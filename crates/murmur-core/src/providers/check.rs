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
}
