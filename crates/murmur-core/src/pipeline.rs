//! audio -> transcript -> vocabulary fixes -> LLM cleanup -> vocabulary fixes.
//!
//! The rule that matters most: a dictation is never lost. If cleanup fails,
//! times out, or misbehaves, the user gets the raw transcript.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::audio::{self, TARGET_RATE};
use crate::config::Settings;
use crate::context::AppContext;
use crate::dictionary;
use crate::error::Result;
use crate::modes::{self, Resolved};
use crate::prompt::{self, PromptSpec};
use crate::providers::llm::{self, Polisher};
use crate::providers::stt::{self, SttRequest, Transcriber};

pub struct Engine {
    pub transcriber: Arc<dyn Transcriber>,
    /// On-device model to fall back on when the cloud fails for good.
    pub fallback: Option<Arc<dyn Transcriber>>,
    pub polisher: Option<Arc<dyn Polisher>>,
}

pub struct Job {
    /// 16 kHz mono.
    pub samples: Vec<f32>,
    pub context: AppContext,
    /// Selected or nearby text in the target field, if the OS layer could read it.
    pub surrounding_text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub raw: String,
    /// What to type. Empty means nothing to insert.
    pub text: String,
    pub resolved: Resolved,
    pub audio_ms: u64,
    pub stt_ms: u64,
    pub polish_ms: u64,
    pub polished: bool,
    /// Why the result is degraded or empty, for the overlay.
    pub note: Option<String>,
}

/// Long recordings go up in pieces this long: small uploads finish quickly,
/// a failure costs one piece rather than the whole dictation, and pieces
/// transcribe in parallel.
pub const PIECE_SECS: u32 = 60;
/// Pieces in flight at once (Groq's free tier allows ~20 requests a minute).
const PARALLEL_PIECES: usize = 3;
/// Waits before each retry of a piece that hit a network error, timeout,
/// rate limit or server error.
const RETRY_BACKOFF: [Duration; 2] = [Duration::from_millis(800), Duration::from_millis(2500)];

/// Cleanup gets more time for longer text: 6 s, plus 1 s per 40 words, up to 45 s.
fn polish_deadline(words: usize) -> Duration {
    Duration::from_secs((6 + words as u64 / 40).min(45))
}

impl Engine {
    pub async fn run(&self, job: Job, settings: &Settings) -> Result<Outcome> {
        let resolved = modes::resolve(&job.context, &settings.modes, &settings.styles, settings.polish.enabled);
        let threshold = settings.audio.silence_threshold_dbfs;
        let audio_ms = audio::duration_ms(job.samples.len(), TARGET_RATE);
        let mut out = Outcome {
            raw: String::new(),
            text: String::new(),
            resolved,
            audio_ms,
            stt_ms: 0,
            polish_ms: 0,
            polished: false,
            note: None,
        };

        if !audio::has_speech(&job.samples, TARGET_RATE, threshold) {
            out.note = Some("No speech detected".into());
            return Ok(out);
        }
        let trimmed = audio::trim_silence(&job.samples, TARGET_RATE, threshold, 250);

        let terms = &settings.dictionary.terms;
        let stt_req = SttRequest {
            language: Some(settings.stt.language.clone()).filter(|l| !l.is_empty()),
            prompt: stt::vocabulary_prompt(terms),
        };
        let t0 = Instant::now();
        let (raw, fell_back) = self.transcribe_pieces(trimmed, threshold, &stt_req).await?;
        out.stt_ms = t0.elapsed().as_millis() as u64;
        if fell_back {
            out.note = Some(format!("{} didn't answer; transcribed on this computer", self.transcriber.label()));
        }

        if stt::is_phantom(&raw, audio::duration_ms(trimmed.len(), TARGET_RATE)) {
            out.note = Some("No speech detected".into());
            return Ok(out);
        }
        // Noise captions ("[Music]", "(dog barking)") and stuck repeat loops are
        // recognizer artifacts, not speech.
        let raw = crate::text::collapse_repeats(&crate::text::strip_sound_tags(&raw));
        if raw.is_empty() {
            out.note = Some("No speech detected".into());
            return Ok(out);
        }
        out.raw = raw.clone();
        let fixed = settings.dictionary.apply(&raw, dictionary::DEFAULT_THRESHOLD);

        let words = fixed.split_whitespace().count();
        let polisher = self.polisher.as_ref().filter(|_| out.resolved.polish && words >= settings.polish.min_words);
        let mut text = fixed.clone();
        if let Some(p) = polisher {
            let system = prompt::system_prompt(&PromptSpec {
                category: Some(out.resolved.category),
                style: out.resolved.style,
                mode_instructions: out.resolved.instructions.as_deref(),
                vocabulary: terms,
                surrounding_text: job.surrounding_text.as_deref(),
                exact: settings.polish.exact,
            });
            let t1 = Instant::now();
            let result = tokio::time::timeout(polish_deadline(words), p.complete(&system, &prompt::user_message(&fixed))).await;
            out.polish_ms = t1.elapsed().as_millis() as u64;
            match result {
                Ok(Ok(cleaned)) => match llm::sanitize(&fixed, &cleaned) {
                    Some(clean) if settings.polish.exact && !crate::text::faithful(&fixed, &clean, terms) => {
                        // It reworded. The speaker's own words win; basic tidy-up runs below.
                        log::warn!("cleanup changed the wording; using the exact transcript instead");
                    }
                    Some(clean) => {
                        // The LLM can reintroduce a misspelling; snap again.
                        text = settings.dictionary.apply(&clean, dictionary::DEFAULT_THRESHOLD);
                        out.polished = true;
                    }
                    None => out.note = Some("Cleanup skipped (unexpected output)".into()),
                },
                Ok(Err(e)) => {
                    log::warn!("cleanup failed: {e}");
                    out.note = Some(format!("Cleanup skipped: {}", e.user_message()));
                }
                Err(_) => out.note = Some("Cleanup timed out; inserted raw text".into()),
            }
        }

        if !out.polished {
            let prose = out.resolved.category != crate::context::Category::Code;
            text = crate::text::tidy(&text, prose);
        }
        out.text = text.trim().to_owned();
        Ok(out)
    }
}

impl Engine {
    /// Transcribe in pieces (see [`PIECE_SECS`]), a few at a time, retrying
    /// transient failures and falling back to the on-device model. Returns
    /// the joined text and whether the fallback was used.
    async fn transcribe_pieces(&self, samples: &[f32], threshold: f32, req: &SttRequest) -> Result<(String, bool)> {
        let pieces = audio::split_for_upload(samples, TARGET_RATE, PIECE_SECS);
        if pieces.len() > 1 {
            log::info!("transcribing {} s of audio in {} pieces", samples.len() / TARGET_RATE as usize, pieces.len());
        }
        let gate = Arc::new(tokio::sync::Semaphore::new(PARALLEL_PIECES));
        let mut tasks = tokio::task::JoinSet::new();
        for (i, piece) in pieces.iter().enumerate() {
            // A long pause can make a whole piece silent; sending it would only invite "Thank you."
            if pieces.len() > 1 && !audio::has_speech(piece, TARGET_RATE, threshold) {
                continue;
            }
            let wav = audio::encode_wav(piece, TARGET_RATE);
            let (primary, fallback, req, gate) = (self.transcriber.clone(), self.fallback.clone(), req.clone(), gate.clone());
            tasks.spawn(async move {
                let _slot = gate.acquire_owned().await;
                (i, transcribe_piece(primary, fallback, wav, &req, i).await)
            });
        }
        let mut texts = vec![String::new(); pieces.len()];
        let mut fell_back = false;
        while let Some(done) = tasks.join_next().await {
            let (i, result) = done.map_err(|e| crate::Error::BadResponse("transcription".into(), e.to_string()))?;
            let (text, fb) = result?;
            texts[i] = text;
            fell_back |= fb;
        }
        let joined = texts.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
        Ok((joined, fell_back))
    }
}

async fn transcribe_piece(
    primary: Arc<dyn Transcriber>,
    fallback: Option<Arc<dyn Transcriber>>,
    wav: Vec<u8>,
    req: &SttRequest,
    index: usize,
) -> Result<(String, bool)> {
    let mut attempt = 0;
    let err = loop {
        match primary.transcribe(wav.clone(), req).await {
            Ok(text) => return Ok((text, false)),
            Err(e) if e.is_transient() && attempt < RETRY_BACKOFF.len() => {
                log::warn!("transcription piece {index}: {e}; retrying");
                tokio::time::sleep(RETRY_BACKOFF[attempt]).await;
                attempt += 1;
            }
            Err(e) => break e,
        }
    };
    log::warn!("transcription piece {index} failed: {err}");
    match fallback {
        Some(local) => {
            log::info!("transcription piece {index}: using the on-device model");
            local.transcribe(wav, req).await.map(|t| (t, true)).map_err(|_| err)
        }
        None => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct FakeStt(String);
    #[async_trait]
    impl Transcriber for FakeStt {
        async fn transcribe(&self, _wav: Vec<u8>, _r: &SttRequest) -> Result<String> {
            Ok(self.0.clone())
        }
        fn label(&self) -> String {
            "fake".into()
        }
    }

    enum Behavior {
        Echo(&'static str),
        Fail,
        Hang,
    }
    struct FakeLlm {
        behavior: Behavior,
        seen_system: Mutex<String>,
    }
    #[async_trait]
    impl Polisher for FakeLlm {
        async fn complete(&self, system: &str, _user: &str) -> Result<String> {
            *self.seen_system.lock().unwrap() = system.to_owned();
            match self.behavior {
                Behavior::Echo(s) => Ok(s.to_owned()),
                Behavior::Fail => Err(Error::Status { provider: "X".into(), status: 500, body: String::new() }),
                Behavior::Hang => {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    unreachable!()
                }
            }
        }
        fn label(&self) -> String {
            "fake".into()
        }
    }

    fn speech() -> Vec<f32> {
        (0..16_000).map(|i| 0.2 * (i as f32 * 0.09).sin()).collect()
    }

    fn engine(raw: &str, llm: Option<Arc<FakeLlm>>) -> Engine {
        Engine {
            transcriber: Arc::new(FakeStt(raw.into())),
            fallback: None,
            polisher: llm.map(|l| l as Arc<dyn Polisher>),
        }
    }

    fn slack() -> AppContext {
        AppContext { app_name: Some("Slack".into()), ..Default::default() }
    }

    fn job(ctx: AppContext) -> Job {
        Job { samples: speech(), context: ctx, surrounding_text: None }
    }

    #[tokio::test]
    async fn happy_path_uses_cleanup_with_app_context() {
        let llm = Arc::new(FakeLlm { behavior: Behavior::Echo("Can you push it to github?"), seen_system: Mutex::new(String::new()) });
        let mut s = Settings::default();
        s.polish.exact = false;
        s.dictionary.terms.push("GitHub".into());
        let out = engine("um can you push it to git hub", Some(llm.clone())).run(job(slack()), &s).await.unwrap();
        assert_eq!(out.text, "Can you push it to GitHub?");
        assert!(out.polished);
        let sys = llm.seen_system.lock().unwrap().clone();
        assert!(sys.contains("work chat"));
        assert!(sys.contains("GitHub"));
    }

    #[tokio::test]
    async fn failure_and_timeout_fall_back_to_raw() {
        let s = Settings::default();
        let fail = Arc::new(FakeLlm { behavior: Behavior::Fail, seen_system: Mutex::new(String::new()) });
        let out = engine("ship it on friday please team", Some(fail)).run(job(slack()), &s).await.unwrap();
        assert_eq!(out.text, "Ship it on friday please team");
        assert!(!out.polished && out.note.is_some());

        tokio::time::pause();
        let hang = Arc::new(FakeLlm { behavior: Behavior::Hang, seen_system: Mutex::new(String::new()) });
        let out = engine("ship it on friday please team", Some(hang)).run(job(slack()), &s).await.unwrap();
        assert_eq!(out.text, "Ship it on friday please team");
        assert!(out.note.unwrap().contains("timed out"));
    }

    #[tokio::test]
    async fn answer_instead_of_cleanup_is_rejected() {
        let llm = Arc::new(FakeLlm {
            behavior: Behavior::Echo("Sure! Here is a detailed poem about the ocean, with waves crashing on the shore and gulls overhead, verse after verse after verse after verse."),
            seen_system: Mutex::new(String::new()),
        });
        let out = engine("write a poem about the ocean", Some(llm)).run(job(slack()), &Settings::default()).await.unwrap();
        assert_eq!(out.text, "Write a poem about the ocean");
    }

    #[tokio::test]
    async fn silence_and_phantoms_insert_nothing() {
        let e = engine("Thank you.", None);
        let silent = Job { samples: vec![0.0; 16_000], context: slack(), surrounding_text: None };
        assert_eq!(e.run(silent, &Settings::default()).await.unwrap().text, "");
        let short: Vec<f32> = speech().into_iter().take(8_000).collect();
        let out = e.run(Job { samples: short, context: slack(), surrounding_text: None }, &Settings::default()).await.unwrap();
        assert_eq!(out.text, "");
        assert_eq!(out.note.as_deref(), Some("No speech detected"));
    }

    #[tokio::test]
    async fn short_dictations_skip_the_llm() {
        let llm = Arc::new(FakeLlm { behavior: Behavior::Fail, seen_system: Mutex::new(String::new()) });
        let out = engine("sounds good", Some(llm.clone())).run(job(slack()), &Settings::default()).await.unwrap();
        assert_eq!(out.text, "Sounds good");
        assert!(out.note.is_none());
        assert!(llm.seen_system.lock().unwrap().is_empty());
    }

    /// Fails the first `failures` calls with `error`, then answers "piece N"
    /// where N counts successful calls.
    struct FlakyStt {
        failures: Mutex<u32>,
        error: fn() -> Error,
        calls: std::sync::atomic::AtomicU32,
        ok: std::sync::atomic::AtomicU32,
    }
    impl FlakyStt {
        fn new(failures: u32, error: fn() -> Error) -> Arc<Self> {
            Arc::new(Self { failures: Mutex::new(failures), error, calls: Default::default(), ok: Default::default() })
        }
    }
    #[async_trait]
    impl Transcriber for FlakyStt {
        async fn transcribe(&self, _wav: Vec<u8>, _r: &SttRequest) -> Result<String> {
            use std::sync::atomic::Ordering::SeqCst;
            self.calls.fetch_add(1, SeqCst);
            {
                let mut f = self.failures.lock().unwrap();
                if *f > 0 {
                    *f -= 1;
                    return Err((self.error)());
                }
            }
            Ok(format!("piece {}", self.ok.fetch_add(1, SeqCst)))
        }
        fn label(&self) -> String {
            "Groq".into()
        }
    }

    fn long_speech(secs: usize) -> Vec<f32> {
        // Speech with a short pause every 5 s, so the splitter has places to cut.
        (0..16_000 * secs).map(|i| if (i / 16_000) % 5 == 4 && i % 16_000 < 4_000 { 0.0 } else { 0.2 * (i as f32 * 0.09).sin() }).collect()
    }

    #[tokio::test(start_paused = true)]
    async fn five_minute_dictation_goes_up_in_pieces_and_keeps_order() {
        let stt = FlakyStt::new(0, || unreachable!());
        let e = Engine { transcriber: stt.clone(), fallback: None, polisher: None };
        let out = e.run(Job { samples: long_speech(300), context: slack(), surrounding_text: None }, &Settings::default()).await.unwrap();
        let calls = stt.calls.load(std::sync::atomic::Ordering::SeqCst);
        assert!((5..=6).contains(&calls), "{calls} pieces");
        // Every piece made it into the text.
        for n in 0..calls {
            assert!(out.raw.contains(&format!("piece {n}")), "{}", out.raw);
        }
        assert!(out.note.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn network_blips_are_retried() {
        let stt = FlakyStt::new(2, || Error::Network("connection reset".into()));
        let e = Engine { transcriber: stt.clone(), fallback: None, polisher: None };
        let out = e.run(job(slack()), &Settings::default()).await.unwrap();
        assert_eq!(out.raw, "piece 0");
        assert_eq!(stt.calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn bad_request_is_not_retried() {
        let stt = FlakyStt::new(5, || Error::Status { provider: "Groq".into(), status: 400, body: String::new() });
        let e = Engine { transcriber: stt.clone(), fallback: None, polisher: None };
        assert!(e.run(job(slack()), &Settings::default()).await.is_err());
        assert_eq!(stt.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn cloud_outage_falls_back_to_the_local_model() {
        let cloud = FlakyStt::new(99, || Error::Timeout("Groq transcription".into(), Duration::from_secs(60)));
        let e = Engine { transcriber: cloud, fallback: Some(Arc::new(FakeStt("said locally".into()))), polisher: None };
        let out = e.run(job(slack()), &Settings::default()).await.unwrap();
        assert_eq!(out.raw, "said locally");
        assert!(out.note.unwrap().contains("on this computer"));
    }

    #[test]
    fn cleanup_gets_more_time_for_long_text() {
        assert_eq!(polish_deadline(20), Duration::from_secs(6));
        assert_eq!(polish_deadline(800), Duration::from_secs(26));
        assert_eq!(polish_deadline(10_000), Duration::from_secs(45));
    }

    #[tokio::test]
    async fn exact_words_keeps_punctuation_cleanup_and_rejects_rewrites() {
        let s = Settings::default();
        assert!(s.polish.exact, "exact words is the default");
        let ok = Arc::new(FakeLlm { behavior: Behavior::Echo("So, can you push it to GitHub before Friday?"), seen_system: Mutex::new(String::new()) });
        let out = engine("um so can you push it to git hub before friday", Some(ok.clone())).run(job(slack()), &s).await.unwrap();
        assert_eq!(out.text, "So, can you push it to GitHub before Friday?");
        assert!(out.polished);
        assert!(ok.seen_system.lock().unwrap().starts_with("You add punctuation"));

        let rewriter = Arc::new(FakeLlm {
            behavior: Behavior::Echo("Could you please push this to GitHub by Friday? Thanks so much!"),
            seen_system: Mutex::new(String::new()),
        });
        let out = engine("um so can you push it to git hub before friday", Some(rewriter)).run(job(slack()), &s).await.unwrap();
        assert!(!out.polished);
        assert_eq!(out.text, "So can you push it to git hub before friday");
    }

    #[tokio::test]
    async fn noise_captions_and_loops_never_reach_the_page() {
        let out = engine("[Music] okay (dog barking) ship it ship it ship it ship it", None).run(job(slack()), &Settings::default()).await.unwrap();
        assert_eq!(out.raw, "okay ship it ship it");
    }
}
