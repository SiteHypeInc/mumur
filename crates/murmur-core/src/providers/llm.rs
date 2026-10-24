//! The cleanup model. Two wire formats cover nearly everything: Anthropic's
//! Messages API and OpenAI-compatible chat completions (OpenAI, Groq,
//! Cerebras, OpenRouter, Ollama, LM Studio, llama.cpp server).

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::providers::stt::truncate;

#[async_trait]
pub trait Polisher: Send + Sync {
    async fn complete(&self, system: &str, user: &str) -> Result<String>;
    fn label(&self) -> String;
}

fn map_send_err(e: reqwest::Error, label: &str, timeout: Duration) -> Error {
    if e.is_timeout() {
        Error::Timeout(format!("{label} cleanup"), timeout)
    } else {
        e.into()
    }
}

async fn check(resp: reqwest::Response, label: &str) -> Result<Value> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Status { provider: label.to_owned(), status: status.as_u16(), body: truncate(&body, 300) });
    }
    resp.json().await.map_err(|e| Error::BadResponse(label.to_owned(), e.to_string()))
}

#[derive(Debug, Clone)]
pub struct AnthropicPolisher {
    client: reqwest::Client,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout: Duration,
}

pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-haiku-5-5";
const ANTHROPIC_VERSION: &str = "2026-10-21";

impl AnthropicPolisher {
    pub fn new(api_key: String, model: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: "https://api.anthropic.com".into(),
            api_key,
            model: model.to_owned(),
            timeout: Duration::from_secs(8),
        }
    }
}

#[async_trait]
impl Polisher for AnthropicPolisher {
    async fn complete(&self, system: &str, user: &str) -> Result<String> {
        // Cleanup is a latency-critical rewrite, not a reasoning task: no
        // thinking, lowest effort. Sampling params are not sent; current models reject them.
        let body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system,
            "messages": [{"role": "user", "content": user}],
            "thinking": {"type": "disabled"},
            "output_config": {"effort": "low"},
        });
        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url.trim_end_matches('/')))
            .timeout(self.timeout)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|e| map_send_err(e, "Anthropic", self.timeout))?;
        let v = check(resp, "Anthropic").await?;
        if v["stop_reason"] == "refusal" {
            return Err(Error::BadResponse("Anthropic".into(), "request declined".into()));
        }
        let text: String = v["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b["type"] == "text")
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        Ok(text)
    }

    fn label(&self) -> String {
        format!("Anthropic · {}", self.model)
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatiblePolisher {
    client: reqwest::Client,
    pub label: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub timeout: Duration,
}

impl OpenAiCompatiblePolisher {
    pub fn new(label: &str, base_url: &str, api_key: Option<String>, model: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            label: label.to_owned(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key: api_key.filter(|k| !k.trim().is_empty()),
            model: model.to_owned(),
            timeout: Duration::from_secs(8),
        }
    }
}

#[async_trait]
impl Polisher for OpenAiCompatiblePolisher {
    async fn complete(&self, system: &str, user: &str) -> Result<String> {
        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "max_tokens": 4096,
            "stream": false,
        });
        let mut req = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .timeout(self.timeout)
            .json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.map_err(|e| map_send_err(e, &self.label, self.timeout))?;
        let v = check(resp, &self.label).await?;
        v["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| Error::BadResponse(self.label.clone(), "no message content".into()))
    }

    fn label(&self) -> String {
        format!("{} · {}", self.label, self.model)
    }
}

/// Undo the ways models decorate output despite instructions, and refuse
/// output that looks like an answer instead of a cleanup. Returns None when
/// the raw transcript should be used instead.
pub fn sanitize(raw_transcript: &str, output: &str) -> Option<String> {
    let mut s = output.to_owned();
    // Reasoning models may leak a think block.
    while let (Some(a), Some(b)) = (s.find("<think>"), s.find("</think>")) {
        if b < a {
            break;
        }
        s.replace_range(a..b + "</think>".len(), "");
    }
    let mut s = s.trim().to_owned();
    for tag in ["transcript", "output", "cleaned", "text"] {
        let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
        if s.starts_with(&open) && s.ends_with(&close) {
            s = s[open.len()..s.len() - close.len()].trim().to_owned();
        }
    }
    if s.starts_with("```") && s.ends_with("```") && s.len() >= 6 {
        s = s.trim_start_matches("```").trim_end_matches("```").trim().to_owned();
        if let Some(nl) = s.find('\n') {
            // Drop a language tag line like ```text.
            if !s[..nl].contains(' ') && s[..nl].len() < 12 {
                s = s[nl + 1..].trim().to_owned();
            }
        }
    }
    for pre in ["Here is the cleaned transcript:", "Here's the cleaned transcript:", "Cleaned transcript:", "Here is the cleaned text:", "Here's the cleaned text:"] {
        if let Some(rest) = s.strip_prefix(pre) {
            s = rest.trim().to_owned();
        }
    }
    let quoted = |t: &str| (t.starts_with('"') && t.ends_with('"')) || (t.starts_with('“') && t.ends_with('”'));
    if quoted(&s) && !quoted(raw_transcript.trim()) && s.chars().count() >= 2 {
        let inner: String = s.chars().skip(1).take(s.chars().count() - 2).collect();
        s = inner.trim().to_owned();
    }

    let raw_len = raw_transcript.trim().chars().count();
    let out_len = s.chars().count();
    if s.is_empty() {
        // Empty is legitimate only when the input was all filler.
        let raw_words = raw_transcript.split_whitespace().count();
        return if raw_words <= 3 { Some(String::new()) } else { None };
    }
    // Cleanup shrinks text or keeps it about the same. A big growth means the
    // model answered the dictation instead of cleaning it.
    if out_len > raw_len * 2 + 80 {
        return None;
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn anthropic_wire_format() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "sk-ant-test"))
            .and(header("anthropic-version", ANTHROPIC_VERSION))
            .and(body_partial_json(json!({"model": "claude-haiku-5-5", "system": "SYS", "thinking": {"type": "disabled"}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": "Hello there."}],
                "stop_reason": "end_turn"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let mut p = AnthropicPolisher::new("sk-ant-test".into(), ANTHROPIC_DEFAULT_MODEL);
        p.base_url = server.uri();
        assert_eq!(p.complete("SYS", "u").await.unwrap(), "Hello there.");
    }

    #[tokio::test]
    async fn anthropic_refusal_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"content": [], "stop_reason": "refusal"})))
            .mount(&server)
            .await;
        let mut p = AnthropicPolisher::new("k".into(), ANTHROPIC_DEFAULT_MODEL);
        p.base_url = server.uri();
        assert!(p.complete("s", "u").await.is_err());
    }

    #[tokio::test]
    async fn openai_compatible_wire_format() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer gsk"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content": "Done."}}]
            })))
            .mount(&server)
            .await;
        let p = OpenAiCompatiblePolisher::new("Groq", &format!("{}/v1", server.uri()), Some("gsk".into()), "m");
        assert_eq!(p.complete("s", "u").await.unwrap(), "Done.");
    }

    #[test]
    fn sanitize_strips_decoration() {
        assert_eq!(sanitize("hi", "<think>hmm</think>\nHi.").as_deref(), Some("Hi."));
        assert_eq!(sanitize("hi", "<transcript>\nHi.\n</transcript>").as_deref(), Some("Hi."));
        assert_eq!(sanitize("hi", "\"Hi.\"").as_deref(), Some("Hi."));
        assert_eq!(sanitize("\"hi\"", "\"Hi.\"").as_deref(), Some("\"Hi.\""));
        assert_eq!(sanitize("hi", "Here's the cleaned text: Hi.").as_deref(), Some("Hi."));
        assert_eq!(sanitize("hi", "```\nHi.\n```").as_deref(), Some("Hi."));
    }

    #[test]
    fn sanitize_rejects_answers_and_bad_empties() {
        let q = "what is the capital of france";
        let answer = "The capital of France is Paris. It has been the capital since the 10th century and is known for the Eiffel Tower, the Louvre and much more.";
        assert_eq!(sanitize(q, answer), None);
        assert_eq!(sanitize("so I was thinking we should ship on friday", ""), None);
        assert_eq!(sanitize("um uh", "").as_deref(), Some(""));
    }
}
