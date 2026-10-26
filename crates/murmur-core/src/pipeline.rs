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

const POLISH_DEADLINE: Duration = Duration::from_secs(6);

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
        let wav = audio::encode_wav(trimmed, TARGET_RATE);

        let terms = &settings.dictionary.terms;
        let stt_req = SttRequest {
            language: Some(settings.stt.language.clone()).filter(|l| !l.is_empty()),
            prompt: stt::vocabulary_prompt(terms),
        };
        let t0 = Instant::now();
        let raw = self.transcriber.transcribe(wav, &stt_req).await?;
        out.stt_ms = t0.elapsed().as_millis() as u64;

        if stt::is_phantom(&raw, audio::duration_ms(trimmed.len(), TARGET_RATE)) {
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
            });
            let t1 = Instant::now();
            let result = tokio::time::timeout(POLISH_DEADLINE, p.complete(&system, &prompt::user_message(&fixed))).await;
            out.polish_ms = t1.elapsed().as_millis() as u64;
            match result {
                Ok(Ok(cleaned)) => match llm::sanitize(&fixed, &cleaned) {
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
}
