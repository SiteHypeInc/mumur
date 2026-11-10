use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Network(String),
    #[error("{provider} returned HTTP {status}: {body}")]
    Status { provider: String, status: u16, body: String },
    #[error("{0} timed out after {1:?}")]
    Timeout(String, Duration),
    #[error("unexpected response from {0}: {1}")]
    BadResponse(String, String),
    #[error("not configured: {0}")]
    NotConfigured(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("invalid settings: {0}")]
    Settings(String),
}

impl Error {
    /// Worth retrying: the network, a timeout, a rate limit or a server hiccup.
    /// A rejected key or a bad request will fail the same way again.
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Network(_) | Error::Timeout(..) => true,
            Error::Status { status, .. } => matches!(status, 408 | 409 | 425 | 429 | 500..=599),
            _ => false,
        }
    }

    /// Short, user-facing explanation for the overlay pill.
    pub fn user_message(&self) -> String {
        match self {
            Error::Status { status: 401 | 403, provider, .. } => format!("{provider} rejected the API key"),
            Error::Status { status: 429, provider, .. } => format!("{provider} rate limit reached"),
            Error::Status { status: 404, provider, body } if body.contains("not found") && body.contains("model") => {
                format!("{provider} model not found")
            }
            Error::Status { provider, status, body } => match api_message(body) {
                Some(m) => format!("{provider} error ({status}): {m}"),
                None => format!("{provider} error ({status})"),
            },
            Error::Timeout(what, _) => format!("{what} took too long"),
            Error::Network(_) => "No connection".into(),
            Error::NotConfigured(what) => format!("Set up {what} in Settings"),
            other => other.to_string(),
        }
    }
}

/// The human part of an API error body (`{"error": {"message": "..."}}`), so
/// the pill can say *why* instead of just a status code.
fn api_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let m = v.pointer("/error/message").or_else(|| v.get("error")).and_then(|m| m.as_str())?;
    let m = m.trim();
    (!m.is_empty()).then(|| if m.chars().count() > 120 { m.chars().take(120).collect::<String>() + "…" } else { m.to_owned() })
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Network(e.to_string())
    }
}

#[cfg(feature = "history")]
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Storage(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_model_says_so() {
        let e = Error::Status {
            provider: "Ollama".into(),
            status: 404,
            body: r#"{"error":{"message":"model \"llama3.2\" not found, try pulling it first"}}"#.into(),
        };
        assert_eq!(e.user_message(), "Ollama model not found");
        let e = Error::Status { provider: "Groq".into(), status: 404, body: "no route".into() };
        assert_eq!(e.user_message(), "Groq error (404)");
        let e = Error::Status {
            provider: "Groq".into(),
            status: 400,
            body: r#"{"error":{"message":"file must be one of the following types","type":"invalid_request_error"}}"#.into(),
        };
        assert_eq!(e.user_message(), "Groq error (400): file must be one of the following types");
        assert!(!e.is_transient());
        assert!(Error::Status { provider: "Groq".into(), status: 503, body: String::new() }.is_transient());
        assert!(Error::Network("reset".into()).is_transient());
    }
}
