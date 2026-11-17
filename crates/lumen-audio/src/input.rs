//! Live audio capture.
//!
//! Two kinds of source:
//! - **Devices** via cpal: audio interfaces, BlackHole, and on Windows the
//!   "(loopback)" of an output device.
//! - **Apps** (macOS 14.2+): capture rekordbox's own output directly with a
//!   Core Audio process tap. See [`crate::mac_tap`].
//!
//! Either way the engine gets an [`AudioFeed`] (thread-safe handles to the
//! analyser and health counters); the [`AudioInput`] that owns the actual
//! stream stays on the UI thread and stops the capture when dropped.

use crate::analyzer::{Analyzer, Wake};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use lumen_core::AudioBands;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Counters updated from the audio thread so the probe can tell whether
/// sound is really arriving (a stream can open fine and deliver silence,
/// e.g. when macOS permission was refused).
#[derive(Default)]
pub struct StreamHealth {
    pub callbacks: AtomicU64,
    pub frames: AtomicU64,
    /// Peak absolute sample since last read, stored as f32 bits.
    peak_bits: AtomicU32,
    /// Milliseconds since `started` of the last callback that contained non-zero audio.
    pub last_sound_ms: AtomicU64,
    /// Time from capture to our callback, as reported by the driver (µs).
    pub input_latency_us: AtomicU64,
    pub errors: AtomicU64,
    pub last_error: Mutex<Option<String>>,
}

impl StreamHealth {
    pub(crate) fn note_peak(&self, p: f32) {
        let mut cur = self.peak_bits.load(Ordering::Relaxed);
        while p > f32::from_bits(cur) {
            match self.peak_bits.compare_exchange_weak(cur, p.to_bits(), Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(c) => cur = c,
            }
        }
    }

    /// Peak since the last call, then reset.
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak_bits.swap(0f32.to_bits(), Ordering::Relaxed))
    }

    pub(crate) fn note_error(&self, e: String) {
        self.errors.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut l) = self.last_error.lock() {
            *l = Some(e);
        }
    }

    /// Book-keeping for one buffer of mono samples, then analyse it.
    pub(crate) fn deliver(&self, started: Instant, analyzer: &Mutex<Analyzer>, mono: &[f32]) {
        let peak = mono.iter().fold(0f32, |m, s| m.max(s.abs()));
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        self.frames.fetch_add(mono.len() as u64, Ordering::Relaxed);
        self.note_peak(peak);
        if peak > 1e-5 {
            self.last_sound_ms.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
        }
        if let Ok(mut a) = analyzer.lock() {
            a.process(mono);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedKind {
    /// An input device (interface, BlackHole...).
    Device,
    /// Recording what an output device plays (Windows WASAPI loopback).
    Loopback,
    /// Captured straight from an app's output (macOS process tap).
    App,
}

/// Everything the light engine needs from a running capture. Cheap to clone,
/// safe to send to another thread.
#[derive(Clone)]
pub struct AudioFeed {
    pub analyzer: Arc<Mutex<Analyzer>>,
    pub health: Arc<StreamHealth>,
    pub started: Instant,
    pub name: String,
    pub kind: FeedKind,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
}

impl AudioFeed {
    pub(crate) fn new(name: String, kind: FeedKind, sample_rate: u32, channels: u16, sample_format: String, wake: Option<Arc<Wake>>) -> Self {
        let mut a = Analyzer::new(sample_rate as f32);
        a.wake = wake;
        AudioFeed {
            analyzer: Arc::new(Mutex::new(a)),
            health: Arc::new(StreamHealth::default()),
            started: Instant::now(),
            name,
            kind,
            sample_rate,
            channels,
            sample_format,
        }
    }

    /// A feed you push samples into yourself (tests, network audio, plug-ins).
    pub fn manual(name: &str, sample_rate: u32, wake: Option<Arc<Wake>>) -> Self {
        AudioFeed::new(name.into(), FeedKind::Device, sample_rate, 1, "f32".into(), wake)
    }

    /// Analyse a buffer of mono samples, as a live stream would.
    pub fn push(&self, mono: &[f32]) {
        self.health.deliver(self.started, &self.analyzer, mono);
    }

    pub fn bands(&self) -> AudioBands {
        self.analyzer.lock().map(|a| a.bands()).unwrap_or_default()
    }

    pub fn label(&self) -> String {
        match self.kind {
            FeedKind::Device => self.name.clone(),
            FeedKind::Loopback => format!("{} (loopback)", self.name),
            FeedKind::App => format!("{} (direct)", self.name),
        }
    }
}

/// A running capture. Keep it alive on the UI thread; dropping it stops the audio.
pub struct AudioInput {
    pub feed: AudioFeed,
    _keep: Box<dyn std::any::Any>,
}

impl AudioInput {
    pub(crate) fn new(feed: AudioFeed, keep: Box<dyn std::any::Any>) -> Self {
        AudioInput { feed, _keep: keep }
    }

    /// Start capturing from a device. `id = None` uses the default input.
    pub fn start(id: Option<&str>, wake: Option<Arc<Wake>>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = match id {
            Some(id) => {
                let parsed = id.parse().map_err(|e| format!("bad device id: {e}"))?;
                host.device_by_id(&parsed).ok_or("device not found")?
            }
            None => host.default_input_device().ok_or("no default input device")?,
        };
        let loopback = !device.supports_input();
        let config = if loopback { device.default_output_config() } else { device.default_input_config() }.map_err(|e| e.to_string())?;
        let feed = AudioFeed::new(
            device.to_string(),
            if loopback { FeedKind::Loopback } else { FeedKind::Device },
            config.sample_rate(),
            config.channels(),
            config.sample_format().to_string(),
            wake,
        );
        let stream = match config.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config.config(), feed.clone()),
            SampleFormat::I16 => build::<i16>(&device, config.config(), feed.clone()),
            SampleFormat::I32 => build::<i32>(&device, config.config(), feed.clone()),
            SampleFormat::U16 => build::<u16>(&device, config.config(), feed.clone()),
            other => Err(format!("unsupported sample format {other}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(AudioInput::new(feed, Box::new(stream)))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    /// Capturing from an output device (records what is being played).
    pub loopback: bool,
}

impl DeviceInfo {
    pub fn label(&self) -> String {
        if self.loopback {
            format!("{} (loopback)", self.name)
        } else {
            self.name.clone()
        }
    }
}

pub fn list_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let mut out = Vec::new();
    let Ok(devices) = host.devices() else { return out };
    for d in devices {
        let Ok(id) = d.id() else { continue };
        let name = d.to_string();
        if d.supports_input() {
            out.push(DeviceInfo { id: id.to_string(), name, loopback: false });
        } else if cfg!(target_os = "windows") && d.supports_output() {
            out.push(DeviceInfo { id: id.to_string(), name, loopback: true });
        }
    }
    out
}

fn build<T>(device: &cpal::Device, config: cpal::StreamConfig, feed: AudioFeed) -> Result<cpal::Stream, String>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let channels = config.channels.max(1) as usize;
    let mut mono: Vec<f32> = Vec::new();
    let err_health = feed.health.clone();
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], info| {
                let ts = info.timestamp();
                let lat = ts.callback.duration_since(ts.capture);
                feed.health.input_latency_us.store(lat.as_micros() as u64, Ordering::Relaxed);
                mono.clear();
                mono.extend(
                    data.chunks(channels).map(|frame| frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32),
                );
                feed.health.deliver(feed.started, &feed.analyzer, &mono);
            },
            move |e| err_health.note_error(e.to_string()),
            None,
        )
        .map_err(|e| e.to_string())
}

// keep the trait in scope for f32::from_sample
#[allow(unused_imports)]
use Sample as _;

/// Default input device name, if any.
pub fn default_input_name() -> Option<String> {
    cpal::default_host().default_input_device().map(|d| d.to_string())
}

pub fn host_name() -> String {
    format!("{:?}", cpal::default_host().id())
}
