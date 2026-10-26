#![cfg_attr(not(feature = "local"), allow(unused))]
//! On-device engine: model list, downloads with progress, and a warm cache.
//! Compiled out without the `local` feature (cross-target type checks).

use std::path::PathBuf;
use std::sync::Arc;

use murmur_core::providers::stt::Transcriber;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Clone)]
pub struct ModelStatus {
    pub id: &'static str,
    pub label: &'static str,
    pub size_mb: u32,
    pub speed: u8,
    pub accuracy: u8,
    pub english_only: bool,
    pub note: &'static str,
    pub downloaded: bool,
}

#[derive(Serialize, Clone)]
struct Progress<'a> {
    id: &'a str,
    done: u64,
    total: Option<u64>,
}

pub const DEFAULT_MODEL: &str = "large-v3-turbo-q5";

pub struct LocalEngine {
    pub dir: PathBuf,
    #[cfg(feature = "local")]
    cache: Arc<murmur_local::ModelCache>,
    downloading: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl LocalEngine {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            #[cfg(feature = "local")]
            cache: Arc::default(),
            downloading: Default::default(),
        }
    }

    pub fn model_id(configured: &str) -> &str {
        if configured.trim().is_empty() { DEFAULT_MODEL } else { configured.trim() }
    }
}

#[cfg(feature = "local")]
impl LocalEngine {
    pub fn list(&self) -> Vec<ModelStatus> {
        murmur_local::MODELS
            .iter()
            .map(|m| ModelStatus {
                id: m.id,
                label: m.label,
                size_mb: m.size_mb,
                speed: m.speed,
                accuracy: m.accuracy,
                english_only: m.english_only,
                note: m.note,
                downloaded: murmur_local::is_downloaded(&self.dir, m),
            })
            .collect()
    }

    pub fn is_ready(&self, id: &str) -> bool {
        murmur_local::model(id).is_some_and(|m| murmur_local::is_downloaded(&self.dir, m))
    }

    pub fn transcriber(&self, id: &str) -> Result<Arc<dyn Transcriber>, String> {
        let m = murmur_local::model(id).ok_or_else(|| format!("Unknown model \"{id}\""))?;
        if !murmur_local::is_downloaded(&self.dir, m) {
            return Err(format!("Download the {} model in Settings → Engines", m.label));
        }
        Ok(Arc::new(murmur_local::LocalWhisper {
            path: murmur_local::model_path(&self.dir, m),
            cache: self.cache.clone(),
            english_only: m.english_only,
        }))
    }

    /// Load the model in the background so the first dictation isn't slow.
    pub fn prewarm(&self, id: &str) {
        let Some(m) = murmur_local::model(id) else { return };
        if !murmur_local::is_downloaded(&self.dir, m) {
            return;
        }
        let path = murmur_local::model_path(&self.dir, m);
        let cache = self.cache.clone();
        std::thread::spawn(move || {
            if let Err(e) = cache.get(&path) {
                log::warn!("prewarm failed: {e}");
            }
        });
    }

    pub async fn download(&self, app: &AppHandle, id: &str) -> Result<(), String> {
        let m = murmur_local::model(id).ok_or_else(|| format!("Unknown model \"{id}\""))?;
        if !self.downloading.lock().unwrap().insert(id.to_owned()) {
            return Err("Already downloading".into());
        }
        let url = format!("{}/{}", murmur_local::download::BASE_URL, m.file);
        let dest = murmur_local::model_path(&self.dir, m);
        let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let app2 = app.clone();
        let result = murmur_local::download::fetch(&url, &dest, move |done, total| {
            // ~10 updates/s is plenty for a progress bar.
            if last_emit.elapsed().as_millis() >= 100 || total == Some(done) {
                last_emit = std::time::Instant::now();
                let _ = app2.emit("model-progress", Progress { id: m.id, done, total });
            }
        })
        .await;
        self.downloading.lock().unwrap().remove(id);
        result.map_err(|e| e.user_message())
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let m = murmur_local::model(id).ok_or_else(|| format!("Unknown model \"{id}\""))?;
        self.cache.unload();
        let path = murmur_local::model_path(&self.dir, m);
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(not(feature = "local"))]
impl LocalEngine {
    pub fn list(&self) -> Vec<ModelStatus> {
        Vec::new()
    }
    pub fn is_ready(&self, _id: &str) -> bool {
        false
    }
    pub fn transcriber(&self, _id: &str) -> Result<Arc<dyn Transcriber>, String> {
        Err("This build has no on-device engine; choose a cloud provider".into())
    }
    pub fn prewarm(&self, _id: &str) {}
    pub async fn download(&self, _app: &AppHandle, _id: &str) -> Result<(), String> {
        Err("This build has no on-device engine".into())
    }
    pub fn delete(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }
}
