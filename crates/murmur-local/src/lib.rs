//! Free, private, on-device transcription with whisper.cpp.
//!
//! Models are downloaded once (Hugging Face, the whisper.cpp project's own
//! mirror) and kept loaded so dictations after the first cost no load time.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use murmur_core::error::{Error, Result};
use murmur_core::providers::stt::{SttRequest, Transcriber};
use serde::Serialize;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub mod download;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ModelInfo {
    pub id: &'static str,
    pub file: &'static str,
    pub label: &'static str,
    pub size_mb: u32,
    /// 1 (fastest) .. 5 (most accurate); for the picker.
    pub speed: u8,
    pub accuracy: u8,
    pub english_only: bool,
    pub note: &'static str,
}

/// Ordered fastest → most accurate. Sizes are the quantized/ggml file sizes.
pub const MODELS: &[ModelInfo] = &[
    ModelInfo { id: "tiny.en", file: "ggml-tiny.en.bin", label: "Tiny (English)", size_mb: 75, speed: 5, accuracy: 1, english_only: true,
        note: "Instant on anything; fine for short notes" },
    ModelInfo { id: "base.en", file: "ggml-base.en.bin", label: "Base (English)", size_mb: 142, speed: 4, accuracy: 2, english_only: true,
        note: "Fast; good for quick messages" },
    ModelInfo { id: "small.en", file: "ggml-small.en.bin", label: "Small (English)", size_mb: 466, speed: 3, accuracy: 3, english_only: true,
        note: "Balanced; solid accuracy on older Macs and PCs" },
    ModelInfo { id: "large-v3-turbo-q5", file: "ggml-large-v3-turbo-q5_0.bin", label: "Large v3 Turbo", size_mb: 547, speed: 3, accuracy: 5, english_only: false,
        note: "Best accuracy, 99 languages. Recommended on Apple Silicon" },
    ModelInfo { id: "small", file: "ggml-small.bin", label: "Small (multilingual)", size_mb: 466, speed: 3, accuracy: 3, english_only: false,
        note: "Balanced, 99 languages" },
];

pub const DEFAULT_MODEL: &str = "large-v3-turbo-q5";

pub fn model(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == id)
}

pub fn model_path(dir: &Path, m: &ModelInfo) -> PathBuf {
    dir.join(m.file)
}

pub fn is_downloaded(dir: &Path, m: &ModelInfo) -> bool {
    download::looks_like_ggml(&model_path(dir, m))
}

/// One loaded model at a time, shared across dictations.
#[derive(Default)]
pub struct ModelCache {
    loaded: Mutex<Option<(PathBuf, Arc<WhisperContext>)>>,
}

impl ModelCache {
    pub fn get(&self, path: &Path) -> Result<Arc<WhisperContext>> {
        let mut guard = self.loaded.lock().unwrap();
        if let Some((p, ctx)) = guard.as_ref() {
            if p == path {
                return Ok(ctx.clone());
            }
        }
        *guard = None; // free the old model before loading a new one
        let t = Instant::now();
        let mut params = WhisperContextParameters::default();
        params.use_gpu(true);
        params.flash_attn(cfg!(target_os = "macos"));
        let ctx = WhisperContext::new_with_params(path, params)
            .map_err(|e| Error::NotConfigured(format!("the speech model ({e}); try downloading it again")))?;
        log::info!("loaded {} in {} ms", path.display(), t.elapsed().as_millis());
        let ctx = Arc::new(ctx);
        *guard = Some((path.to_owned(), ctx.clone()));
        Ok(ctx)
    }

    pub fn unload(&self) {
        *self.loaded.lock().unwrap() = None;
    }
}

pub struct LocalWhisper {
    pub path: PathBuf,
    pub cache: Arc<ModelCache>,
    pub english_only: bool,
}

/// Our pipeline hands transcribers 16-bit mono WAV; turn it back into samples.
fn wav_to_f32(wav: &[u8]) -> Vec<f32> {
    let data = wav.windows(4).position(|w| w == b"data").map(|i| i + 8).unwrap_or(44).min(wav.len());
    wav[data..].as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b) as f32 / 32768.0).collect()
}

fn threads() -> i32 {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8) as i32
}

impl LocalWhisper {
    fn run(&self, samples: Vec<f32>, req: SttRequest) -> Result<String> {
        let ctx = self.cache.get(&self.path)?;
        let mut state = ctx.create_state().map_err(|e| Error::BadResponse("whisper".into(), e.to_string()))?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let lang = req.language.as_deref().filter(|l| !l.is_empty() && *l != "auto");
        params.set_language(if self.english_only { Some("en") } else { lang.or(Some("auto")) });
        if let Some(p) = req.prompt.as_deref() {
            params.set_initial_prompt(p);
        }
        params.set_n_threads(threads());
        params.set_no_context(true);
        params.set_suppress_blank(true);
        // No "[Music]" / "(dog barking)" captions: non-speech tokens are suppressed.
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        // whisper.cpp needs at least 1 s of audio; pad short clips with silence.
        let mut samples = samples;
        if samples.len() < 16_000 + 1_600 {
            samples.resize(16_000 + 1_600, 0.0);
        }
        state.full(params, &samples).map_err(|e| Error::BadResponse("whisper".into(), e.to_string()))?;
        let text: String = state
            .as_iter()
            .filter_map(|seg| seg.to_str_lossy().ok().map(|s| s.into_owned()))
            .collect::<Vec<_>>()
            .join("");
        Ok(text.trim().to_owned())
    }
}

#[async_trait]
impl Transcriber for LocalWhisper {
    async fn transcribe(&self, wav: Vec<u8>, req: &SttRequest) -> Result<String> {
        let samples = wav_to_f32(&wav);
        let me = LocalWhisper { path: self.path.clone(), cache: self.cache.clone(), english_only: self.english_only };
        let req = req.clone();
        tokio::task::spawn_blocking(move || me.run(samples, req))
            .await
            .map_err(|e| Error::BadResponse("whisper".into(), e.to_string()))?
    }

    fn label(&self) -> String {
        format!("On-device · {}", self.path.file_name().and_then(|f| f.to_str()).unwrap_or("whisper"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_roundtrip() {
        let wav = murmur_core::audio::encode_wav(&[0.0, 0.5, -0.5], 16_000);
        let back = wav_to_f32(&wav);
        assert_eq!(back.len(), 3);
        assert!((back[1] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn catalog_is_ordered_and_default_exists() {
        assert!(model(DEFAULT_MODEL).is_some());
        assert!(MODELS.windows(2).take(3).all(|w| w[0].size_mb <= w[1].size_mb));
    }

    #[test]
    fn missing_model_is_a_clear_error() {
        let w = LocalWhisper { path: "/nonexistent/ggml.bin".into(), cache: Arc::default(), english_only: true };
        let err = w.run(vec![0.0; 16_000], SttRequest::default()).unwrap_err();
        assert!(err.user_message().contains("speech model"), "{}", err.user_message());
    }
}
