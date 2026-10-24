//! User settings, persisted as TOML. API keys are not stored here; the app
//! keeps them in the OS keychain.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::context::Category;
use crate::dictionary::Dictionary;
use crate::error::{Error, Result};
use crate::hotkey::ActivationMode;
use crate::modes::{default_styles, Mode, Style};
use crate::providers::llm::{AnthropicPolisher, OpenAiCompatiblePolisher, Polisher, ANTHROPIC_DEFAULT_MODEL};
use crate::providers::stt::{OpenAiCompatibleStt, Transcriber};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SttProvider {
    #[default]
    Groq,
    OpenAi,
    /// Any OpenAI-compatible server, e.g. a local whisper.cpp `whisper-server`.
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolishProvider {
    #[default]
    Anthropic,
    Groq,
    OpenAi,
    /// Any OpenAI-compatible server, e.g. Ollama or LM Studio for fully local cleanup.
    Custom,
}

impl SttProvider {
    pub fn default_base_url(self) -> &'static str {
        match self {
            SttProvider::Groq => "https://api.groq.com/openai/v1",
            SttProvider::OpenAi => "https://api.openai.com/v1",
            SttProvider::Custom => "http://127.0.0.1:8080/v1",
        }
    }
    pub fn default_model(self) -> &'static str {
        match self {
            SttProvider::Groq => "whisper-large-v3-turbo",
            SttProvider::OpenAi => "whisper-1",
            SttProvider::Custom => "whisper-1",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SttProvider::Groq => "Groq",
            SttProvider::OpenAi => "OpenAI",
            SttProvider::Custom => "Custom",
        }
    }
}

impl PolishProvider {
    pub fn default_base_url(self) -> &'static str {
        match self {
            PolishProvider::Anthropic => "https://api.anthropic.com",
            PolishProvider::Groq => "https://api.groq.com/openai/v1",
            PolishProvider::OpenAi => "https://api.openai.com/v1",
            PolishProvider::Custom => "http://127.0.0.1:11434/v1",
        }
    }
    pub fn default_model(self) -> &'static str {
        match self {
            PolishProvider::Anthropic => ANTHROPIC_DEFAULT_MODEL,
            PolishProvider::Groq => "llama-3.3-70b-versatile",
            PolishProvider::OpenAi => "gpt-4.1-mini",
            PolishProvider::Custom => "llama3.2",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            PolishProvider::Anthropic => "Anthropic",
            PolishProvider::Groq => "Groq",
            PolishProvider::OpenAi => "OpenAI",
            PolishProvider::Custom => "Custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeySettings {
    /// "Fn", "RightOption", "RightCommand", "RightControl", or a combo like "Ctrl+Shift+Space".
    pub binding: String,
    pub mode: ActivationMode,
    pub hold_threshold_ms: u64,
    /// Pressed while recording to discard it.
    pub cancel: String,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            binding: if cfg!(target_os = "macos") { "Fn".into() } else { "Ctrl+Win".into() },
            mode: ActivationMode::Hybrid,
            hold_threshold_ms: 350,
            cancel: "Escape".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SttSettings {
    pub provider: SttProvider,
    /// Empty = provider default.
    pub model: String,
    pub base_url: String,
    /// ISO-639-1, empty/"auto" = detect.
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PolishSettings {
    pub enabled: bool,
    pub provider: PolishProvider,
    pub model: String,
    pub base_url: String,
    /// Below this many words, skip the LLM: nothing to clean, and it saves ~300 ms.
    pub min_words: usize,
}

impl Default for PolishSettings {
    fn default() -> Self {
        Self { enabled: true, provider: PolishProvider::Anthropic, model: String::new(), base_url: String::new(), min_words: 4 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PasteSettings {
    pub restore_clipboard: bool,
    pub restore_delay_ms: u64,
    pub trailing_space: bool,
}

impl Default for PasteSettings {
    fn default() -> Self {
        Self { restore_clipboard: true, restore_delay_ms: 400, trailing_space: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// Input device name; empty = system default.
    pub device: String,
    pub sounds: bool,
    /// Frames quieter than this count as silence.
    pub silence_threshold_dbfs: f32,
    pub max_recording_secs: u64,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self { device: String::new(), sounds: true, silence_threshold_dbfs: -50.0, max_recording_secs: 600 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub hotkey: HotkeySettings,
    pub stt: SttSettings,
    pub polish: PolishSettings,
    pub styles: HashMap<Category, Style>,
    pub modes: Vec<Mode>,
    pub dictionary: Dictionary,
    pub learn_from_edits: bool,
    pub paste: PasteSettings,
    pub audio: AudioSettings,
    /// Number of dictations kept in history; 0 disables history.
    pub history_limit: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: HotkeySettings::default(),
            stt: SttSettings::default(),
            polish: PolishSettings::default(),
            styles: default_styles(),
            modes: Vec::new(),
            dictionary: Dictionary::default(),
            learn_from_edits: true,
            paste: PasteSettings::default(),
            audio: AudioSettings::default(),
            history_limit: 500,
        }
    }
}

fn or_default(value: &str, default: &str) -> String {
    let v = value.trim();
    if v.is_empty() { default.to_owned() } else { v.to_owned() }
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| Error::Settings(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(Error::Settings(e.to_string())),
        }
    }

    /// Write atomically so a crash mid-save never leaves a truncated file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let s = toml::to_string_pretty(self).map_err(|e| Error::Settings(e.to_string()))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::Settings(e.to_string()))?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, s).map_err(|e| Error::Settings(e.to_string()))?;
        std::fs::rename(&tmp, path).map_err(|e| Error::Settings(e.to_string()))
    }

    pub fn build_transcriber(&self, api_key: Option<String>) -> Result<Arc<dyn Transcriber>> {
        let p = self.stt.provider;
        if p != SttProvider::Custom && api_key.as_deref().is_none_or(|k| k.trim().is_empty()) {
            return Err(Error::NotConfigured(format!("a {} API key for transcription", p.label())));
        }
        Ok(Arc::new(OpenAiCompatibleStt::new(
            p.label(),
            &or_default(&self.stt.base_url, p.default_base_url()),
            api_key,
            &or_default(&self.stt.model, p.default_model()),
        )))
    }

    /// None when cleanup is off or has no key (dictation still works, raw).
    pub fn build_polisher(&self, api_key: Option<String>) -> Option<Arc<dyn Polisher>> {
        if !self.polish.enabled {
            return None;
        }
        let p = self.polish.provider;
        let key = api_key.filter(|k| !k.trim().is_empty());
        let model = or_default(&self.polish.model, p.default_model());
        match p {
            PolishProvider::Anthropic => {
                let mut a = AnthropicPolisher::new(key?, &model);
                a.base_url = or_default(&self.polish.base_url, p.default_base_url());
                Some(Arc::new(a))
            }
            PolishProvider::Custom => Some(Arc::new(OpenAiCompatiblePolisher::new(
                p.label(),
                &or_default(&self.polish.base_url, p.default_base_url()),
                key,
                &model,
            ))),
            _ => Some(Arc::new(OpenAiCompatiblePolisher::new(
                p.label(),
                &or_default(&self.polish.base_url, p.default_base_url()),
                Some(key?),
                &model,
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_toml() {
        let mut s = Settings::default();
        s.dictionary.terms.push("GitHub".into());
        s.modes.push(Mode { name: "Linear".into(), hosts: vec!["linear.app".into()], ..Default::default() });
        s.styles.insert(Category::Personal, Style::VeryCasual);
        let dir = std::env::temp_dir().join(format!("murmur-test-{}", std::process::id()));
        let path = dir.join("settings.toml");
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_file_gives_defaults_and_partial_file_fills_in() {
        assert_eq!(Settings::load(Path::new("/nonexistent/murmur.toml")).unwrap(), Settings::default());
        let s: Settings = toml::from_str("[polish]\nenabled = false\n").unwrap();
        assert!(!s.polish.enabled);
        assert_eq!(s.polish.min_words, 4);
        assert_eq!(s.paste, PasteSettings::default());
    }

    #[test]
    fn providers_need_keys() {
        let s = Settings::default();
        assert!(s.build_transcriber(None).is_err());
        assert!(s.build_transcriber(Some("gsk".into())).is_ok());
        assert!(s.build_polisher(None).is_none());
        assert!(s.build_polisher(Some("sk-ant".into())).is_some());
        let mut local = Settings::default();
        local.stt.provider = SttProvider::Custom;
        local.polish.provider = PolishProvider::Custom;
        assert!(local.build_transcriber(None).is_ok());
        assert!(local.build_polisher(None).is_some());
    }
}
