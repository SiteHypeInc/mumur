//! Start/stop cues played natively on the default output device (your
//! earbuds when they're connected). Web audio in a hidden window is blocked by
//! WebKit's autoplay policy on macOS, so cues can't come from the overlay.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    Start,
    Stop,
    Cancel,
    Error,
}

impl Cue {
    /// (frequency Hz, duration ms) per note.
    fn notes(self) -> &'static [(f32, u32)] {
        match self {
            Cue::Start => &[(660.0, 70), (880.0, 90)],
            Cue::Stop => &[(880.0, 70), (660.0, 90)],
            Cue::Cancel => &[(440.0, 110)],
            Cue::Error => &[(330.0, 100), (262.0, 140)],
        }
    }
}

const VOLUME: f32 = 0.18;

/// Mono samples for a cue, with a short attack and exponential decay per note
/// so there are no clicks.
pub fn render(cue: Cue, rate: u32) -> Vec<f32> {
    let mut out = Vec::new();
    for &(freq, ms) in cue.notes() {
        let n = (rate as u64 * ms as u64 / 1000) as usize;
        let attack = (rate as usize / 200).max(1); // 5 ms
        for i in 0..n {
            let t = i as f32 / rate as f32;
            let env = if i < attack { i as f32 / attack as f32 } else { (-6.0 * (i - attack) as f32 / n as f32).exp() };
            out.push(VOLUME * env * (2.0 * std::f32::consts::PI * freq * t).sin());
        }
    }
    // A little silence so the stream doesn't stop mid-decay.
    out.extend(std::iter::repeat_n(0.0, rate as usize / 20));
    out
}

/// Fire and forget.
pub fn play(cue: Cue) {
    std::thread::spawn(move || {
        if let Err(e) = play_blocking(cue) {
            log::warn!("sound cue failed: {e}");
        }
    });
}

fn play_blocking(cue: Cue) -> Result<(), String> {
    let device = cpal::default_host().default_output_device().ok_or("no output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let samples = render(cue, rate);
    let duration = std::time::Duration::from_millis((samples.len() as u64 * 1000) / rate as u64 + 60);

    fn build<T: SizedSample + cpal::FromSample<f32> + Send + 'static>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        samples: Vec<f32>,
        channels: usize,
    ) -> Result<cpal::Stream, String> {
        let mut pos = 0usize;
        device
            .build_output_stream(
                config,
                move |data: &mut [T], _| {
                    for frame in data.chunks_mut(channels) {
                        let v = samples.get(pos).copied().unwrap_or(0.0);
                        pos += 1;
                        for s in frame {
                            *s = T::from_sample(v);
                        }
                    }
                },
                |e| log::warn!("output stream error: {e}"),
                None,
            )
            .map_err(|e| e.to_string())
    }

    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &config, samples, channels)?,
        SampleFormat::I16 => build::<i16>(&device, &config, samples, channels)?,
        SampleFormat::U16 => build::<u16>(&device, &config, samples, channels)?,
        SampleFormat::I32 => build::<i32>(&device, &config, samples, channels)?,
        other => return Err(format!("unsupported output format {other:?}")),
    };
    stream.play().map_err(|e| e.to_string())?;
    std::thread::sleep(duration);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cues_are_short_quiet_and_click_free() {
        for cue in [Cue::Start, Cue::Stop, Cue::Cancel, Cue::Error] {
            let s = render(cue, 48_000);
            let ms = s.len() * 1000 / 48_000;
            assert!((100..400).contains(&ms), "{cue:?} {ms} ms");
            assert!(s.iter().all(|v| v.abs() <= VOLUME + 1e-6));
            assert!(s[0].abs() < 0.01, "starts at zero");
            assert!(s.last().unwrap().abs() < 1e-6, "ends silent");
        }
    }
}
