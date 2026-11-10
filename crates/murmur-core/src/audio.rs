//! Sample-level helpers: downmix, resample to 16 kHz, silence detection, WAV.

pub const TARGET_RATE: u32 = 16_000;

/// Average interleaved channels into mono.
pub fn to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    if ch == 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

/// Resample mono audio. Downsampling averages each output sample's source
/// window (a box filter), which keeps aliasing out of the speech band without
/// pulling in a DSP crate; upsampling interpolates linearly.
pub fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = ((input.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    if ratio > 1.0 {
        for i in 0..out_len {
            let start = (i as f64 * ratio) as usize;
            let end = (((i + 1) as f64 * ratio) as usize).min(input.len()).max(start + 1);
            let window = &input[start..end];
            out.push(window.iter().sum::<f32>() / window.len() as f32);
        }
    } else {
        for i in 0..out_len {
            let pos = i as f64 * ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx];
            let b = *input.get(idx + 1).unwrap_or(&a);
            out.push(a + (b - a) * frac);
        }
    }
    out
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

pub fn rms_dbfs(samples: &[f32]) -> f32 {
    20.0 * rms(samples).max(1e-9).log10()
}

/// Energy-based voice gate. Whisper-family models hallucinate on silence
/// ("Thank you.", "Thanks for watching!"), so a recording with no frame
/// louder than `threshold_dbfs` never reaches the transcriber.
pub fn has_speech(samples: &[f32], rate: u32, threshold_dbfs: f32) -> bool {
    let frame = (rate as usize / 50).max(1); // 20 ms
    let loud_frames = samples
        .chunks(frame)
        .filter(|f| rms_dbfs(f) > threshold_dbfs)
        .count();
    // Require ~100 ms of energy, so a click or a key thump doesn't count.
    loud_frames >= 5
}

/// Trim leading/trailing quiet, keeping `pad_ms` of context on each side so
/// soft consonants at word edges survive. Less audio = faster upload + decode.
pub fn trim_silence(samples: &[f32], rate: u32, threshold_dbfs: f32, pad_ms: u32) -> &[f32] {
    let frame = (rate as usize / 50).max(1);
    let frames: Vec<bool> = samples.chunks(frame).map(|f| rms_dbfs(f) > threshold_dbfs).collect();
    let Some(first) = frames.iter().position(|&l| l) else {
        return &samples[0..0];
    };
    let last = frames.iter().rposition(|&l| l).unwrap_or(first);
    let pad = (rate as usize * pad_ms as usize) / 1000;
    let start = (first * frame).saturating_sub(pad);
    let end = ((last + 1) * frame + pad).min(samples.len());
    &samples[start..end]
}

/// All-zero input: not a quiet room but a blocked or dead microphone (macOS
/// hands apps without mic permission pure digital silence).
pub fn is_digital_silence(samples: &[f32]) -> bool {
    !samples.is_empty() && samples.iter().all(|s| s.abs() < 1e-6)
}

pub fn duration_ms(samples: usize, rate: u32) -> u64 {
    (samples as u64 * 1000) / rate.max(1) as u64
}

/// 16-bit PCM mono WAV.
pub fn encode_wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Read back a WAV written by [`encode_wav`] (16-bit PCM mono). Used to retry
/// a recording saved to disk. Returns (samples, rate).
pub fn decode_wav(bytes: &[u8]) -> Option<(Vec<f32>, u32)> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let rate = u32::from_le_bytes(bytes[24..28].try_into().ok()?);
    let channels = u16::from_le_bytes(bytes[22..24].try_into().ok()?);
    let bits = u16::from_le_bytes(bytes[34..36].try_into().ok()?);
    if channels != 1 || bits != 16 {
        return None;
    }
    // Find the data chunk rather than assuming offset 36.
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        if id == b"data" {
            let end = (pos + 8 + len).min(bytes.len());
            let samples = bytes[pos + 8..end]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes(*b) as f32 / i16::MAX as f32)
                .collect();
            return Some((samples, rate));
        }
        pos += 8 + len + (len & 1);
    }
    None
}

/// Split a long recording into pieces of at most `max_secs`, cutting at the
/// quietest moment in the last few seconds before each limit so words aren't
/// sliced in half. Short recordings come back as one piece.
pub fn split_for_upload(samples: &[f32], rate: u32, max_secs: u32) -> Vec<&[f32]> {
    let max = (rate * max_secs) as usize;
    if max == 0 || samples.len() <= max {
        return vec![samples];
    }
    let frame = (rate as usize / 10).max(1); // 100 ms
    let search = (rate as usize * 8).min(max / 2); // look back up to 8 s
    let mut out = Vec::new();
    let mut start = 0;
    while samples.len() - start > max {
        let limit = start + max;
        let from = limit - search;
        let cut = (from..limit - frame)
            .step_by(frame)
            .min_by(|&a, &b| rms(&samples[a..a + frame]).total_cmp(&rms(&samples[b..b + frame])))
            .map(|q| q + frame / 2)
            .unwrap_or(limit);
        out.push(&samples[start..cut]);
        start = cut;
    }
    out.push(&samples[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, ms: u32, amp: f32) -> Vec<f32> {
        let n = rate as usize * ms as usize / 1000;
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn mono_averages_channels() {
        assert_eq!(to_mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
    }

    #[test]
    fn resample_48k_to_16k_keeps_duration_and_tone() {
        let input = sine(440.0, 48_000, 1000, 0.5);
        let out = resample(&input, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
        // 440 Hz is far below the box filter's first null, so energy is retained.
        assert!((rms(&out) - rms(&input)).abs() < 0.02, "{} vs {}", rms(&out), rms(&input));
    }

    #[test]
    fn resample_attenuates_content_above_new_nyquist() {
        let input = sine(15_000.0, 48_000, 500, 0.5);
        let out = resample(&input, 48_000, 16_000);
        assert!(rms(&out) < rms(&input) * 0.5);
    }

    #[test]
    fn upsample_doubles_length() {
        let out = resample(&[0.0, 1.0, 0.0, 1.0], 8_000, 16_000);
        assert_eq!(out.len(), 8);
        assert!((out[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn silence_has_no_speech_and_tone_does() {
        let quiet = vec![0.0005f32; 16_000];
        assert!(!has_speech(&quiet, 16_000, -45.0));
        let voice = sine(220.0, 16_000, 500, 0.2);
        assert!(has_speech(&voice, 16_000, -45.0));
        let click = sine(220.0, 16_000, 40, 0.8);
        assert!(!has_speech(&click, 16_000, -45.0));
    }

    #[test]
    fn trim_keeps_speech_plus_padding() {
        let mut s = vec![0.0f32; 16_000];
        s.extend(sine(220.0, 16_000, 500, 0.2));
        s.extend(vec![0.0f32; 16_000]);
        let t = trim_silence(&s, 16_000, -45.0, 100);
        let ms = duration_ms(t.len(), 16_000);
        assert!((680..=740).contains(&ms), "{ms}");
    }

    #[test]
    fn digital_silence_vs_quiet_room() {
        assert!(is_digital_silence(&[0.0; 1000]));
        assert!(!is_digital_silence(&[0.0005; 1000]));
        assert!(!is_digital_silence(&[]));
    }

    #[test]
    fn wav_header_is_valid() {
        let wav = encode_wav(&[0.0, 1.0, -1.0], 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 44 + 6);
        assert_eq!(i16::from_le_bytes([wav[46], wav[47]]), i16::MAX);
    }

    #[test]
    fn wav_round_trip() {
        let samples: Vec<f32> = (0..1600).map(|i| ((i as f32) / 50.0).sin() * 0.5).collect();
        let (back, rate) = decode_wav(&encode_wav(&samples, 16_000)).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(back.len(), samples.len());
        assert!(back.iter().zip(&samples).all(|(a, b)| (a - b).abs() < 1e-3));
        assert!(decode_wav(b"not a wav").is_none());
    }

    #[test]
    fn splits_long_audio_at_pauses() {
        let rate = 16_000;
        // 5 minutes of "speech" with a one-second pause every 5 s.
        let samples: Vec<f32> = (0..rate as usize * 300)
            .map(|i| if (i / rate as usize) % 5 == 4 { 0.0 } else { ((i as f32) / 7.0).sin() * 0.3 })
            .collect();
        let parts = split_for_upload(&samples, rate, 60);
        assert!(parts.len() >= 5, "{} parts", parts.len());
        assert_eq!(parts.iter().map(|p| p.len()).sum::<usize>(), samples.len());
        assert!(parts.iter().all(|p| p.len() <= rate as usize * 60));
        // Every cut lands inside a quiet gap.
        let mut at = 0;
        for p in &parts[..parts.len() - 1] {
            at += p.len();
            assert!(samples[at].abs() < 1e-6, "cut at {at} is not in a pause");
        }
        // Short clips stay whole.
        assert_eq!(split_for_upload(&samples[..rate as usize * 30], rate, 60).len(), 1);
    }
}
