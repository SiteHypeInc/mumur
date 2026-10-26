//! Transcribes real speech with a real whisper model. Needs a model file:
//!   MURMUR_TEST_MODEL=/path/to/ggml-tiny.en.bin cargo test -p murmur-local --test real_model
//! Skips (passes) when the variable isn't set, so `cargo test` works offline.

use std::sync::Arc;

use murmur_core::config::Settings;
use murmur_core::context::AppContext;
use murmur_core::pipeline::{Engine, Job};
use murmur_core::providers::stt::{SttRequest, Transcriber};
use murmur_local::{LocalWhisper, ModelCache};

fn model() -> Option<std::path::PathBuf> {
    std::env::var_os("MURMUR_TEST_MODEL").map(Into::into)
}

fn jfk_wav() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/jfk.wav")).unwrap()
}

fn engine_for(path: std::path::PathBuf, cache: Arc<ModelCache>) -> LocalWhisper {
    LocalWhisper { path, cache, english_only: true }
}

#[tokio::test]
async fn transcribes_jfk() {
    let Some(path) = model() else {
        eprintln!("MURMUR_TEST_MODEL not set; skipping");
        return;
    };
    let cache = Arc::new(ModelCache::default());
    let w = engine_for(path.clone(), cache.clone());

    let t = std::time::Instant::now();
    let text = w.transcribe(jfk_wav(), &SttRequest::default()).await.unwrap();
    let cold = t.elapsed();
    println!("cold: {cold:?}: {text}");
    let lower = text.to_lowercase();
    assert!(lower.contains("ask not what your country can do for you"), "{text}");

    // Second run reuses the loaded model.
    let t = std::time::Instant::now();
    w.transcribe(jfk_wav(), &SttRequest::default()).await.unwrap();
    println!("warm: {:?}", t.elapsed());
}

#[tokio::test]
async fn full_pipeline_offline_with_vocabulary_and_tidy() {
    let Some(path) = model() else { return };
    // Decode the fixture into 16 kHz samples, as the recorder would deliver them.
    let wav = jfk_wav();
    let data = wav.windows(4).position(|w| w == b"data").unwrap() + 8;
    let samples: Vec<f32> = wav[data..].as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b) as f32 / 32768.0).collect();

    let mut settings = Settings::default();
    settings.dictionary.terms.push("Fellow Americans".into());
    let engine = Engine {
        transcriber: Arc::new(engine_for(path, Arc::default())),
        polisher: None, // the free path: no LLM, built-in tidy-up
    };
    let out = engine
        .run(Job { samples, context: AppContext { app_name: Some("Notes".into()), ..Default::default() }, surrounding_text: None }, &settings)
        .await
        .unwrap();
    println!("{out:?}");
    assert!(out.text.starts_with(|c: char| c.is_uppercase()), "{}", out.text);
    assert!(out.text.to_lowercase().contains("your country"), "{}", out.text);
    assert!(!out.polished);
}
