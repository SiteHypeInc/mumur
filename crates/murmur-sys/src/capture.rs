//! Microphone capture on a dedicated thread (cpal streams are not `Send` on
//! every platform).
//!
//! After a recording the stream stays open for `KEEP_WARM`, continuously
//! keeping the last `PREROLL` of audio. A dictation that starts while warm
//! begins with that pre-roll, so the first syllable is never clipped and there
//! is no device-open delay.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use murmur_core::audio;

const KEEP_WARM: Duration = Duration::from_secs(30);
const PREROLL: Duration = Duration::from_millis(300);
const MAX_BUFFERED_SAMPLES: usize = 48_000 * 2 * 60 * 15;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("no microphone found")]
    NoDevice,
    #[error("microphone error: {0}")]
    Device(String),
    #[error("audio thread stopped")]
    ThreadGone,
}

enum Cmd {
    Start { device: Option<String>, reply: Sender<Result<(), CaptureError>> },
    Stop { reply: Sender<Vec<f32>> },
    Cancel,
}

struct Shared {
    recording: AtomicBool,
    buf: Mutex<Vec<f32>>,
    preroll: Mutex<VecDeque<f32>>,
    preroll_cap: AtomicU32,
    /// Latest input level as f32 bits, 0.0..1.0 (RMS, scaled for display).
    level: AtomicU32,
}

struct Open {
    _stream: Stream,
    device: String,
    rate: u32,
    channels: u16,
    idle_since: Option<Instant>,
}

pub struct Recorder {
    tx: Sender<Cmd>,
    shared: Arc<Shared>,
}

impl Recorder {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<Cmd>();
        let shared = Arc::new(Shared {
            recording: AtomicBool::new(false),
            buf: Mutex::new(Vec::new()),
            preroll: Mutex::new(VecDeque::new()),
            preroll_cap: AtomicU32::new(0),
            level: AtomicU32::new(0),
        });
        let s = shared.clone();
        thread::Builder::new()
            .name("murmur-audio".into())
            .spawn(move || {
                let mut open: Option<Open> = None;
                loop {
                    let cmd = match rx.recv_timeout(Duration::from_secs(1)) {
                        Ok(c) => c,
                        Err(RecvTimeoutError::Timeout) => {
                            if open.as_ref().and_then(|o| o.idle_since).is_some_and(|t| t.elapsed() > KEEP_WARM) {
                                open = None; // closes the device, mic indicator goes off
                                s.preroll.lock().unwrap().clear();
                            }
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    };
                    match cmd {
                        Cmd::Start { device, reply } => {
                            let wanted = device.filter(|d| !d.is_empty());
                            let reuse = open.as_ref().is_some_and(|o| wanted.as_ref().is_none_or(|w| *w == o.device));
                            if !reuse {
                                open = None;
                                match open_stream(wanted.as_deref(), &s) {
                                    Ok(o) => open = Some(o),
                                    Err(e) => {
                                        let _ = reply.send(Err(e));
                                        continue;
                                    }
                                }
                            }
                            let o = open.as_mut().unwrap();
                            o.idle_since = None;
                            {
                                let mut buf = s.buf.lock().unwrap();
                                buf.clear();
                                buf.extend(s.preroll.lock().unwrap().drain(..));
                            }
                            s.recording.store(true, Ordering::SeqCst);
                            let _ = reply.send(Ok(()));
                        }
                        Cmd::Stop { reply } => {
                            s.recording.store(false, Ordering::SeqCst);
                            let raw = std::mem::take(&mut *s.buf.lock().unwrap());
                            let out = match open.as_mut() {
                                Some(o) => {
                                    o.idle_since = Some(Instant::now());
                                    let mono = audio::to_mono(&raw, o.channels);
                                    audio::resample(&mono, o.rate, audio::TARGET_RATE)
                                }
                                None => Vec::new(),
                            };
                            let _ = reply.send(out);
                        }
                        Cmd::Cancel => {
                            s.recording.store(false, Ordering::SeqCst);
                            s.buf.lock().unwrap().clear();
                            if let Some(o) = open.as_mut() {
                                o.idle_since = Some(Instant::now());
                            }
                        }
                    }
                }
            })
            .expect("spawn audio thread");
        Self { tx, shared }
    }

    /// Opens the device if needed and starts buffering. Blocks until the
    /// stream is live, so the caller can play the start sound after it.
    pub fn start(&self, device: Option<String>) -> Result<(), CaptureError> {
        let (reply, rx) = mpsc::channel();
        self.tx.send(Cmd::Start { device, reply }).map_err(|_| CaptureError::ThreadGone)?;
        rx.recv_timeout(Duration::from_secs(5)).map_err(|_| CaptureError::ThreadGone)?
    }

    /// Stops and returns 16 kHz mono samples.
    pub fn stop(&self) -> Vec<f32> {
        let (reply, rx) = mpsc::channel();
        if self.tx.send(Cmd::Stop { reply }).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default()
    }

    pub fn cancel(&self) {
        let _ = self.tx.send(Cmd::Cancel);
    }

    pub fn is_recording(&self) -> bool {
        self.shared.recording.load(Ordering::SeqCst)
    }

    /// 0.0..1.0, for the waveform in the overlay.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.shared.level.load(Ordering::Relaxed))
    }
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

pub fn input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

fn open_stream(wanted: Option<&str>, shared: &Arc<Shared>) -> Result<Open, CaptureError> {
    let host = cpal::default_host();
    let device = match wanted {
        Some(name) => host
            .input_devices()
            .map_err(|e| CaptureError::Device(e.to_string()))?
            .find(|d| d.name().is_ok_and(|n| n == name))
            .or_else(|| host.default_input_device()),
        None => host.default_input_device(),
    }
    .ok_or(CaptureError::NoDevice)?;
    let name = device.name().unwrap_or_default();
    let supported = device.default_input_config().map_err(|e| CaptureError::Device(e.to_string()))?;
    let rate = supported.sample_rate().0;
    let channels = supported.channels();
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    shared
        .preroll_cap
        // Whole frames only, so trimming the ring never splits a frame's channels.
        .store(((rate as u128 * PREROLL.as_millis() / 1000) * channels as u128) as u32, Ordering::Relaxed);

    let err_fn = |e| log::error!("audio stream error: {e}");
    macro_rules! build {
        ($t:ty, $conv:expr) => {{
            let s = shared.clone();
            device.build_input_stream(
                &config,
                move |data: &[$t], _| {
                    let samples: Vec<f32> = data.iter().map($conv).collect();
                    push(&s, &samples);
                },
                err_fn,
                None,
            )
        }};
    }
    let stream = match format {
        SampleFormat::F32 => build!(f32, |&x: &f32| x),
        SampleFormat::I16 => build!(i16, |&x: &i16| x as f32 / i16::MAX as f32),
        SampleFormat::U16 => build!(u16, |&x: &u16| (x as f32 - 32_768.0) / 32_768.0),
        SampleFormat::I32 => build!(i32, |&x: &i32| x as f32 / i32::MAX as f32),
        other => return Err(CaptureError::Device(format!("unsupported sample format {other:?}"))),
    }
    .map_err(|e| CaptureError::Device(e.to_string()))?;
    stream.play().map_err(|e| CaptureError::Device(e.to_string()))?;
    log::info!("opened input \"{name}\" at {rate} Hz, {channels} ch");
    Ok(Open { _stream: stream, device: name, rate, channels, idle_since: None })
}

fn push(s: &Shared, samples: &[f32]) {
    // Speech RMS rarely exceeds 0.3; scale so normal talking fills the meter.
    let level = (audio::rms(samples) * 4.0).min(1.0);
    s.level.store(level.to_bits(), Ordering::Relaxed);
    if s.recording.load(Ordering::Relaxed) {
        if let Ok(mut buf) = s.buf.lock() {
            // Hard cap (~15 min of 48 kHz stereo) so a runaway device can't eat memory.
            if buf.len() + samples.len() <= MAX_BUFFERED_SAMPLES {
                buf.extend_from_slice(samples);
            }
        }
    } else if let Ok(mut pre) = s.preroll.lock() {
        let cap = s.preroll_cap.load(Ordering::Relaxed) as usize;
        pre.extend(samples.iter().copied());
        let excess = pre.len().saturating_sub(cap);
        pre.drain(..excess);
    }
}
