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
    /// Short, user-facing explanation for the overlay pill.
    pub fn user_message(&self) -> String {
        match self {
            Error::Status { status: 401 | 403, provider, .. } => format!("{provider} rejected the API key"),
            Error::Status { status: 429, provider, .. } => format!("{provider} rate limit reached"),
            Error::Status { status: 404, provider, body } if body.contains("not found") && body.contains("model") => {
                format!("{provider} model not found")
            }
            Error::Status { provider, status, .. } => format!("{provider} error ({status})"),
            Error::Timeout(what, _) => format!("{what} took too long"),
            Error::Network(_) => "No connection".into(),
            Error::NotConfigured(what) => format!("Set up {what} in Settings"),
            other => other.to_string(),
        }
    }
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
    }
}
