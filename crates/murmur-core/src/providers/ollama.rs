//! Ollama extras on top of its OpenAI-compatible API: which models are
//! installed, and pulling one, so "model not found" is a click to fix.

use std::time::Duration;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::providers::stt::truncate;

/// Ollama's native API lives at the server root; settings store the `/v1` URL.
fn api_root(base_url: &str) -> String {
    let b = base_url.trim().trim_end_matches('/');
    b.strip_suffix("/v1").unwrap_or(b).to_owned()
}

/// `llama3.2` and `llama3.2:latest` are the same model.
pub fn has_model(installed: &[String], model: &str) -> bool {
    let norm = |m: &str| m.strip_suffix(":latest").unwrap_or(m).to_owned();
    let want = norm(model.trim());
    installed.iter().any(|m| norm(m) == want)
}

async fn check(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(Error::Status { provider: "Ollama".into(), status: status.as_u16(), body: truncate(&body, 200) })
}

/// Names of the installed models, e.g. `["llama3.2:latest"]`.
pub async fn installed(base_url: &str) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Tags {
        #[serde(default)]
        models: Vec<Tag>,
    }
    #[derive(Deserialize)]
    struct Tag {
        name: String,
    }
    let resp = reqwest::Client::new()
        .get(format!("{}/api/tags", api_root(base_url)))
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    let tags: Tags = check(resp).await?.json().await.map_err(|e| Error::BadResponse("Ollama".into(), e.to_string()))?;
    Ok(tags.models.into_iter().map(|t| t.name).collect())
}

/// Download a model into Ollama. Blocks until it's done (can take minutes).
pub async fn pull(base_url: &str, model: &str) -> Result<()> {
    let resp = reqwest::Client::new()
        .post(format!("{}/api/pull", api_root(base_url)))
        .timeout(Duration::from_secs(60 * 60))
        .json(&serde_json::json!({ "model": model.trim(), "stream": false }))
        .send()
        .await?;
    let body: serde_json::Value =
        check(resp).await?.json().await.map_err(|e| Error::BadResponse("Ollama".into(), e.to_string()))?;
    match body.get("error").and_then(|e| e.as_str()) {
        Some(e) => Err(Error::BadResponse("Ollama".into(), e.to_owned())),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn matches_latest_tag() {
        let have = vec!["llama3.2:latest".to_owned(), "qwen2.5:7b".to_owned()];
        assert!(has_model(&have, "llama3.2"));
        assert!(has_model(&have, "llama3.2:latest"));
        assert!(has_model(&have, "qwen2.5:7b"));
        assert!(!has_model(&have, "qwen2.5"));
        assert!(!has_model(&[], "llama3.2"));
        assert_eq!(api_root("http://127.0.0.1:11434/v1/"), "http://127.0.0.1:11434");
    }

    #[tokio::test]
    async fn lists_and_pulls() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"models":[{"name":"llama3.2:latest"}]})))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/api/pull")).and(body_partial_json(serde_json::json!({"model":"llama3.2"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":"success"})))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/api/pull"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"error":"pull model manifest: file does not exist"})))
            .mount(&server).await;
        let base = format!("{}/v1", server.uri());
        assert_eq!(installed(&base).await.unwrap(), ["llama3.2:latest"]);
        assert!(pull(&base, "llama3.2").await.is_ok());
        assert!(pull(&base, "nope").await.unwrap_err().to_string().contains("does not exist"));
    }
}
