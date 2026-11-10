//! Speech-to-text over the OpenAI-compatible `/audio/transcriptions` endpoint.
//! Groq, OpenAI, and local servers (whisper.cpp `whisper-server`, speaches,
//! LocalAI) all speak it, so one client covers cloud and local.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Default)]
pub struct SttRequest {
    /// ISO-639-1 code, or None for auto-detect.
    pub language: Option<String>,
    /// Spelling hints (vocabulary). Whisper reads this as preceding context.
    pub prompt: Option<String>,
}

#[async_trait]
pub trait Transcriber: Send + Sync {
    async fn transcribe(&self, wav: Vec<u8>, req: &SttRequest) -> Result<String>;
    fn label(&self) -> String;
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleStt {
    client: reqwest::Client,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub timeout: Duration,
    pub label: String,
}

impl OpenAiCompatibleStt {
    pub fn new(label: &str, base_url: &str, api_key: Option<String>, model: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key: api_key.filter(|k| !k.trim().is_empty()),
            model: model.to_owned(),
            // Per piece: pieces are at most a minute of audio (~2 MB).
            timeout: Duration::from_secs(60),
            label: label.to_owned(),
        }
    }
}

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}

#[async_trait]
impl Transcriber for OpenAiCompatibleStt {
    async fn transcribe(&self, wav: Vec<u8>, req: &SttRequest) -> Result<String> {
        let part = Part::bytes(wav).file_name("audio.wav").mime_str("audio/wav").map_err(Error::from)?;
        let mut form = Form::new()
            .part("file", part)
            .text("model", self.model.clone())
            .text("response_format", "json")
            .text("temperature", "0");
        if let Some(lang) = req.language.as_ref().filter(|l| !l.is_empty() && *l != "auto") {
            form = form.text("language", lang.clone());
        }
        if let Some(prompt) = req.prompt.as_ref().filter(|p| !p.is_empty()) {
            form = form.text("prompt", prompt.clone());
        }

        let mut http = self
            .client
            .post(format!("{}/audio/transcriptions", self.base_url))
            .timeout(self.timeout)
            .multipart(form);
        if let Some(key) = &self.api_key {
            http = http.bearer_auth(key);
        }
        let resp = http.send().await.map_err(|e| {
            if e.is_timeout() { Error::Timeout(format!("{} transcription", self.label), self.timeout) } else { e.into() }
        })?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Status { provider: self.label.clone(), status: status.as_u16(), body: truncate(&body, 300) });
        }
        let parsed: TranscriptionResponse =
            resp.json().await.map_err(|e| Error::BadResponse(self.label.clone(), e.to_string()))?;
        Ok(parsed.text.trim().to_owned())
    }

    fn label(&self) -> String {
        format!("{} · {}", self.label, self.model)
    }
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    s.chars().take(max).collect::<String>() + "…"
}

/// Build Whisper's `prompt` from vocabulary. Whisper only looks at the last
/// ~224 tokens, so keep it short and put terms in a natural sentence.
pub fn vocabulary_prompt(terms: &[String]) -> Option<String> {
    let mut list = String::new();
    for t in terms.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        if list.len() + t.len() > 600 {
            break;
        }
        if !list.is_empty() {
            list.push_str(", ");
        }
        list.push_str(t);
    }
    (!list.is_empty()).then(|| format!("Vocabulary: {list}."))
}

/// Phrases Whisper-family models emit for near-silence or noise. If the whole
/// transcript is one of these and the clip was short, it's not real speech.
pub fn is_phantom(text: &str, audio_ms: u64) -> bool {
    const PHANTOMS: &[&str] = &[
        "thank you", "thanks", "thank you very much", "thanks for watching", "thank you for watching",
        "thanks for watching!", "you", "bye", "bye bye", "okay", "so", "the end", "subtitles by the amaraorg community",
        "please subscribe", "like and subscribe",
    ];
    let norm: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if norm.is_empty() {
        return true;
    }
    if norm.contains("amaraorg") || norm.contains("subtitles by") {
        return true;
    }
    audio_ms < 2_500 && PHANTOMS.contains(&norm.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn posts_multipart_and_reads_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/audio/transcriptions"))
            .and(header("authorization", "Bearer gsk_test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"text": "  hello world "})))
            .expect(1)
            .mount(&server)
            .await;
        let stt = OpenAiCompatibleStt::new("Groq", &format!("{}/openai/v1/", server.uri()), Some("gsk_test".into()), "whisper-large-v3-turbo");
        let text = stt
            .transcribe(vec![0; 64], &SttRequest { language: Some("en".into()), prompt: Some("Vocabulary: Murmur.".into()) })
            .await
            .unwrap();
        assert_eq!(text, "hello world");
        let body = String::from_utf8_lossy(&server.received_requests().await.unwrap()[0].body).to_string();
        assert!(body.contains("name=\"model\"\r\n\r\nwhisper-large-v3-turbo"));
        assert!(body.contains("name=\"language\"\r\n\r\nen"));
        assert!(body.contains("Vocabulary: Murmur."));
    }

    #[tokio::test]
    async fn surfaces_auth_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("invalid key"))
            .mount(&server)
            .await;
        let stt = OpenAiCompatibleStt::new("Groq", &server.uri(), Some("bad".into()), "m");
        let err = stt.transcribe(vec![], &SttRequest::default()).await.unwrap_err();
        assert_eq!(err.user_message(), "Groq rejected the API key");
    }

    #[test]
    fn phantoms() {
        assert!(is_phantom("Thank you.", 900));
        assert!(!is_phantom("Thank you.", 6_000));
        assert!(is_phantom("Subtitles by the Amara.org community", 30_000));
        assert!(!is_phantom("Thank you for the update on the launch.", 900));
        assert!(is_phantom("  ", 900));
    }

    #[test]
    fn vocab_prompt() {
        assert_eq!(vocabulary_prompt(&["GitHub".into(), "Sinéad".into()]).as_deref(), Some("Vocabulary: GitHub, Sinéad."));
        assert_eq!(vocabulary_prompt(&[]), None);
    }
}
