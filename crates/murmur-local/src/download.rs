//! Resumable model download with progress, written to `<file>.part` and moved
//! into place only after it's complete and looks like a ggml model.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use murmur_core::error::{Error, Result};
use tokio::io::AsyncWriteExt;

pub const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// ggml files start with the magic "lmgg" (0x67676d6c little-endian).
pub fn looks_like_ggml(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return false;
    }
    let big_enough = f.metadata().map(|m| m.len() > 10_000_000).unwrap_or(false);
    magic == *b"lmgg" && big_enough
}

/// Downloads `url` to `dest`, calling `progress(downloaded, total)` as bytes arrive.
/// Resumes a previous partial download when the server supports ranges.
pub async fn fetch(url: &str, dest: &Path, mut progress: impl FnMut(u64, Option<u64>) + Send) -> Result<()> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::Storage(e.to_string()))?;
    }
    let part = dest.with_extension("part");
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(Error::from)?;
    let mut req = client.get(url);
    if have > 0 {
        req = req.header("Range", format!("bytes={have}-"));
    }
    let mut resp = req.send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(Error::Status { provider: "Model download".into(), status: status.as_u16(), body: String::new() });
    }
    let resumed = status.as_u16() == 206;
    let start = if resumed { have } else { 0 };
    let total = resp.content_length().map(|len| len + start);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)
        .await
        .map_err(|e| Error::Storage(e.to_string()))?;
    let mut done = start;
    progress(done, total);
    while let Some(chunk) = resp.chunk().await? {
        file.write_all(&chunk).await.map_err(|e| Error::Storage(e.to_string()))?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    file.flush().await.map_err(|e| Error::Storage(e.to_string()))?;
    drop(file);

    if total.is_some_and(|t| done < t) {
        return Err(Error::Network("download interrupted; it will resume next time".into()));
    }
    if !looks_like_ggml(&part) {
        let _ = std::fs::remove_file(&part);
        return Err(Error::BadResponse("Model download".into(), "the file isn't a whisper model".into()));
    }
    std::fs::rename(&part, dest).map_err(|e| Error::Storage(e.to_string()))?;
    let _ = std::fs::File::open(dest).and_then(|mut f| f.flush());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fake_model(len: usize) -> Vec<u8> {
        let mut v = b"lmgg".to_vec();
        v.resize(len, 7);
        v
    }

    #[tokio::test]
    async fn downloads_validates_and_reports_progress() {
        let server = MockServer::start().await;
        let body = fake_model(12_000_000);
        Mock::given(method("GET")).and(path("/m.bin")).respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone())).mount(&server).await;
        let dir = std::env::temp_dir().join(format!("murmur-dl-{}", std::process::id()));
        let dest = dir.join("m.bin");
        let mut last = (0, None);
        fetch(&format!("{}/m.bin", server.uri()), &dest, |d, t| last = (d, t)).await.unwrap();
        assert!(looks_like_ggml(&dest));
        assert_eq!(last, (12_000_000, Some(12_000_000)));
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn resumes_partial_downloads() {
        let server = MockServer::start().await;
        let body = fake_model(12_000_000);
        Mock::given(method("GET")).and(path("/m.bin")).and(header("range", "bytes=5000000-"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(body[5_000_000..].to_vec())).mount(&server).await;
        let dir = std::env::temp_dir().join(format!("murmur-dl-resume-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("m.bin");
        std::fs::write(dest.with_extension("part"), &body[..5_000_000]).unwrap();
        fetch(&format!("{}/m.bin", server.uri()), &dest, |_, _| {}).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn rejects_non_models() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_string("<html>login</html>")).mount(&server).await;
        let dir = std::env::temp_dir().join(format!("murmur-dl-bad-{}", std::process::id()));
        let dest = dir.join("m.bin");
        assert!(fetch(&format!("{}/m.bin", server.uri()), &dest, |_, _| {}).await.is_err());
        assert!(!dest.exists());
        std::fs::remove_dir_all(dir).ok();
    }
}
