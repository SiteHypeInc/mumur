//! Cheap credential checks for the Settings "Test" button: list models,
//! which every provider supports and which costs nothing.

use std::time::Duration;

use crate::error::{Error, Result};
use crate::providers::stt::truncate;

const TIMEOUT: Duration = Duration::from_secs(8);

async fn finish(resp: reqwest::Response, label: &str) -> Result<()> {
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let body = resp.text().await.unwrap_or_default();
    Err(Error::Status { provider: label.to_owned(), status: status.as_u16(), body: truncate(&body, 200) })
}

pub async fn openai_compatible(label: &str, base_url: &str, api_key: Option<&str>) -> Result<()> {
    let mut req = reqwest::Client::new()
        .get(format!("{}/models", base_url.trim_end_matches('/')))
        .timeout(TIMEOUT);
    if let Some(k) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    finish(req.send().await?, label).await
}

/// Speech-to-text models the server offers (Whisper and "transcribe" models),
/// so the Engines dropdown picks up new ones without an app update.
pub async fn speech_models(label: &str, base_url: &str, api_key: Option<&str>) -> Result<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct List {
        data: Vec<Model>,
    }
    #[derive(serde::Deserialize)]
    struct Model {
        id: String,
    }
    let mut req = reqwest::Client::new()
        .get(format!("{}/models", base_url.trim_end_matches('/')))
        .timeout(TIMEOUT);
    if let Some(k) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        finish(resp, label).await?;
        return Ok(Vec::new());
    }
    let list: List = resp.json().await.map_err(|e| Error::BadResponse(label.to_owned(), e.to_string()))?;
    let mut ids: Vec<String> = list
        .data
        .into_iter()
        .map(|m| m.id)
        .filter(|id| {
            let l = id.to_lowercase();
            (l.contains("whisper") || l.contains("transcribe")) && !l.contains("tts")
        })
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub async fn anthropic(base_url: &str, api_key: &str) -> Result<()> {
    let resp = reqwest::Client::new()
        .get(format!("{}/v1/models", base_url.trim_end_matches('/')))
        .timeout(TIMEOUT)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2026-10-21")
        .send()
        .await?;
    finish(resp, "Anthropic").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn checks_keys() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v1/models")).and(header("x-api-key", "good"))
            .respond_with(ResponseTemplate::new(200)).mount(&server).await;
        Mock::given(method("GET")).and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(401)).mount(&server).await;
        assert!(anthropic(&server.uri(), "good").await.is_ok());
        assert_eq!(anthropic(&server.uri(), "bad").await.unwrap_err().user_message(), "Anthropic rejected the API key");
        Mock::given(method("GET")).and(path("/openai/v1/models")).and(header("authorization", "Bearer gsk"))
            .respond_with(ResponseTemplate::new(200)).mount(&server).await;
        assert!(openai_compatible("Groq", &format!("{}/openai/v1", server.uri()), Some("gsk")).await.is_ok());
    }

    #[tokio::test]
    async fn lists_only_speech_models() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [
                {"id": "whisper-large-v3-turbo"}, {"id": "llama-3.3-70b-versatile"},
                {"id": "whisper-large-v3"}, {"id": "gpt-4o-mini-transcribe"}, {"id": "gpt-4o-mini-tts"}
            ]})))
            .mount(&server).await;
        let ids = speech_models("Groq", &format!("{}/v1", server.uri()), Some("k")).await.unwrap();
        assert_eq!(ids, ["gpt-4o-mini-transcribe", "whisper-large-v3", "whisper-large-v3-turbo"]);
    }
}
