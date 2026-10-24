//! API keys live in the OS keychain (Keychain, Credential Manager, Secret
//! Service), one entry per provider, never in the settings file.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use murmur_core::config::{PolishProvider, SttProvider};

const SERVICE: &str = "app.murmur.dictation";

pub const PROVIDERS: [&str; 4] = ["groq", "openai", "anthropic", "custom"];

pub fn stt_key_name(p: SttProvider) -> &'static str {
    match p {
        SttProvider::Groq => "groq",
        SttProvider::OpenAi => "openai",
        SttProvider::Custom => "custom",
    }
}

pub fn polish_key_name(p: PolishProvider) -> &'static str {
    match p {
        PolishProvider::Anthropic => "anthropic",
        PolishProvider::Groq => "groq",
        PolishProvider::OpenAi => "openai",
        PolishProvider::Custom => "custom",
    }
}

fn entry(provider: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(SERVICE, provider).ok()
}

/// Keychain reads can be slow (or hang until a DBus timeout on Linux without a
/// secret service), so each provider is read once and cached.
fn cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static C: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// `MURMUR_GROQ_API_KEY`-style env vars win over the keychain (CI, power users).
fn from_env(provider: &str) -> Option<String> {
    std::env::var(format!("MURMUR_{}_API_KEY", provider.to_uppercase())).ok().filter(|k| !k.trim().is_empty())
}

pub fn get(provider: &str) -> Option<String> {
    if let Some(k) = from_env(provider) {
        return Some(k);
    }
    if std::env::var_os("MURMUR_NO_KEYCHAIN").is_some() {
        return None;
    }
    if let Some(hit) = cache().lock().unwrap().get(provider) {
        return hit.clone();
    }
    let value = entry(provider).and_then(|e| e.get_password().ok()).filter(|k| !k.trim().is_empty());
    cache().lock().unwrap().insert(provider.to_owned(), value.clone());
    value
}

pub fn set(provider: &str, key: &str) -> Result<(), String> {
    if !PROVIDERS.contains(&provider) {
        return Err(format!("unknown provider {provider}"));
    }
    let e = entry(provider).ok_or("keychain unavailable")?;
    let key = key.trim();
    let result = if key.is_empty() {
        match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    } else {
        e.set_password(key).map_err(|err| err.to_string())
    };
    if result.is_ok() {
        cache().lock().unwrap().insert(provider.to_owned(), (!key.is_empty()).then(|| key.to_owned()));
    }
    result
}
