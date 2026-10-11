//! Live audio for Oorapp: the microphone and what the computer plays, each
//! captured for the whole life of the app, so the meters work before and
//! after a recording too.
//!
//! Both come through cpal. The mic is the default input device; the computer
//! audio is the default output device opened for input, which cpal turns into
//! a Core Audio process tap on macOS (14.4 or newer, "System Audio Recording"
//! permission) and WASAPI loopback on Windows. Whatever the device delivers is
//! converted to the staging format (s16le, 48 kHz, stereo) and handed on in
//! 20 ms chunks, as `parec` does in the upstream Linux recorder, so the
//! timeline logic below is unchanged: a track that stops getting audio for a
//! while catches up with silence and stays in step with the other.
//!
//! When the default device changes (a headset plugged in, AirPods connected)
//! the stream is closed and opened again on the new one.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use oorapp_core::codec::Resampler;
pub use oorapp_core::{CHANNELS, RATE};

/// 20 ms of s16le audio.
const CHUNK_BYTES: usize = (RATE / 50 * 2 * CHANNELS) as usize;
const FRAME_BYTES: usize = (2 * CHANNELS) as usize;
/// A read that waits this long means the device gave nothing in between.
const STALL: Duration = Duration::from_millis(250);
/// Three seconds of 20 ms peaks.
pub const HISTORY: usize = 150;
const FLOOR_DB: f64 = -60.0;
/// How often the default device is looked up, to follow a switch.
const DEVICE_CHECK: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The default input device.
    Mic,
    /// What the default output device plays.
    Computer,
}

struct Inner {
    levels: VecDeque<f32>,
    recording: Option<Recording>,
    /// While paused the meters keep running but nothing is written.
    paused: bool,
    /// Why the device could not be opened, for the UI; None while it works.
    problem: Option<String>,
    /// The device's name, for the UI.
    device: Option<String>,
    /// 20 ms chunks in a row that were exactly zero. A macOS process tap
    /// without the "System Audio Recording" permission delivers only that;
    /// a real room or call never does.
    zero_chunks: u64,
}

struct Recording {
    file: BufWriter<File>,
    /// Where the recording is, so audio that never came becomes silence.
    timeline: Timeline,
}

#[derive(Clone)]
pub struct Source {
    inner: Arc<Mutex<Inner>>,
}

impl Source {
    /// Starts capturing `kind` on a thread of its own, for as long as the app runs.
    pub fn spawn(kind: Kind) -> Self {
        let inner = Arc::new(Mutex::new(Inner {
            levels: VecDeque::from(vec![0.0; HISTORY]),
            recording: None,
            paused: false,
            problem: None,
            device: None,
            zero_chunks: 0,
        }));
        let shared = inner.clone();
        thread::Builder::new()
            .name(format!("capture-{kind:?}"))
            .spawn(move || {
                loop {
                    if let Err(problem) = capture(kind, &shared) {
                        shared.lock().unwrap().problem = Some(problem);
                    }
                    // The device went away or could not be opened; try again.
                    thread::sleep(Duration::from_secs(1));
                }
            })
            .expect("capture thread");
        Source { inner }
    }

    /// Tees the stream (s16le, RATE, CHANNELS) into `path` from now on.
    pub fn start_recording(&self, path: &Path) -> std::io::Result<()> {
        let file = BufWriter::new(File::create(path)?);
        let mut inner = self.inner.lock().unwrap();
        inner.recording = Some(Recording {
            file,
            timeline: Timeline::new(Instant::now()),
        });
        inner.paused = false;
        Ok(())
    }

    pub fn set_paused(&self, paused: bool) {
        let mut inner = self.inner.lock().unwrap();
        inner.paused = paused;
        if let Some(recording) = inner.recording.as_mut() {
            recording.timeline.set_paused(paused, Instant::now());
        }
    }

    pub fn stop_recording(&self) {
        if let Some(mut recording) = self.inner.lock().unwrap().recording.take() {
            let _ = recording.file.flush();
        }
    }

    pub fn levels(&self) -> Vec<f32> {
        self.inner.lock().unwrap().levels.iter().copied().collect()
    }

    /// The loudest of the last `n` peaks, so a short burst is not missed by a slower reader.
    pub fn recent_peak(&self, n: usize) -> f32 {
        let inner = self.inner.lock().unwrap();
        inner
            .levels
            .iter()
            .rev()
            .take(n)
            .copied()
            .fold(0.0, f32::max)
    }

    /// Why this source gets no audio, such as a missing permission.
    pub fn problem(&self) -> Option<String> {
        self.inner.lock().unwrap().problem.clone()
    }

    /// How long this source has delivered nothing but digital zeros. For the
    /// computer audio on macOS, seconds of it while something plays usually
    /// means the "System Audio Recording" permission is missing.
    pub fn digital_silence(&self) -> Duration {
        Duration::from_millis(self.inner.lock().unwrap().zero_chunks * 20)
    }

    /// The name of the device it listens to.
    pub fn device(&self) -> Option<String> {
        self.inner.lock().unwrap().device.clone()
    }
}

fn default_device(host: &cpal::Host, kind: Kind) -> Option<cpal::Device> {
    match kind {
        Kind::Mic => host.default_input_device(),
        Kind::Computer => host.default_output_device(),
    }
}

/// Audio as the device delivers it, interleaved f32.
struct Block(Vec<f32>);

/// Runs one stream on the current default device until it fails or the
/// default device changes. Err when it could not start at all.
fn capture(kind: Kind, shared: &Mutex<Inner>) -> Result<(), String> {
    let host = cpal::default_host();
    let device = default_device(&host, kind).ok_or_else(|| match kind {
        Kind::Mic => "No microphone found".to_owned(),
        Kind::Computer => "No audio output found".to_owned(),
    })?;
    let id = device.id().ok();
    let config = match kind {
        Kind::Mic => device.default_input_config(),
        Kind::Computer => device.default_output_config(),
    }
    .map_err(|e| format!("Could not open {}: {e}", label(&device)))?;
    let (channels, rate) = (usize::from(config.channels()), config.sample_rate());

    let (tx, rx) = std::sync::mpsc::sync_channel::<Block>(256);
    let failed = Arc::new(AtomicBool::new(false));
    let on_error = {
        let failed = failed.clone();
        move |_e: cpal::Error| failed.store(true, Ordering::Relaxed)
    };
    let stream = build_stream(&device, config, tx, on_error)
        .map_err(|e| format!("Could not open {}: {e}", label(&device)))?;
    stream
        .play()
        .map_err(|e| format!("Could not start {}: {e}", label(&device)))?;
    {
        let mut inner = shared.lock().unwrap();
        inner.problem = None;
        inner.device = Some(label(&device));
    }

    let mut converter = Converter::new(channels, rate);
    let mut pending: Vec<u8> = Vec::with_capacity(CHUNK_BYTES * 4);
    let mut chunks: u64 = 0;
    let mut checked = Instant::now();
    let mut asked = Instant::now();
    loop {
        if failed.load(Ordering::Relaxed) {
            return Ok(());
        }
        if checked.elapsed() >= DEVICE_CHECK {
            checked = Instant::now();
            if default_device(&host, kind).and_then(|d| d.id().ok()) != id {
                return Ok(()); // a new default device: start over on it
            }
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Block(samples)) => converter.push(&samples, &mut pending),
            Err(RecvTimeoutError::Timeout) => {
                // Nothing comes while nothing plays (WASAPI loopback): keep
                // the meter falling instead of frozen on the last peak.
                if asked.elapsed() > STALL {
                    let mut inner = shared.lock().unwrap();
                    for _ in 0..5 {
                        inner.levels.pop_front();
                        inner.levels.push_back(0.0);
                    }
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        let mut offset = 0;
        while pending.len() - offset >= CHUNK_BYTES {
            let chunk = &pending[offset..offset + CHUNK_BYTES];
            offset += CHUNK_BYTES;
            chunks += 1;
            let now = Instant::now();
            let waited = now.saturating_duration_since(asked);
            asked = now;
            handle_chunk(shared, chunk, waited, chunks == 1, chunks, now);
        }
        pending.drain(..offset);
    }
}

fn label(device: &cpal::Device) -> String {
    device
        .description()
        .map(|d| d.to_string())
        .or_else(|_| device.id().map(|id| id.to_string()))
        .unwrap_or_else(|_| "the audio device".to_owned())
}

fn build_stream(
    device: &cpal::Device,
    config: cpal::SupportedStreamConfig,
    tx: SyncSender<Block>,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, String> {
    use cpal::SampleFormat as F;
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    // The callback runs on the audio thread: convert to f32 and hand it on,
    // dropping a block rather than blocking when the reader falls behind.
    macro_rules! stream {
        ($t:ty, $to_f32:expr) => {
            device.build_input_stream(
                config,
                move |data: &[$t], _: &cpal::InputCallbackInfo| {
                    let block: Vec<f32> = data.iter().map($to_f32).collect();
                    let _ = tx.try_send(Block(block));
                },
                on_error,
                None,
            )
            .map_err(|e| e.to_string())
        };
    }
    match format {
        F::F32 => stream!(f32, |v: &f32| *v),
        F::I16 => stream!(i16, |v: &i16| f32::from(*v) / 32768.0),
        F::I32 => stream!(i32, |v: &i32| *v as f32 / 2_147_483_648.0),
        F::U16 => stream!(u16, |v: &u16| (f32::from(*v) - 32768.0) / 32768.0),
        F::F64 => stream!(f64, |v: &f64| *v as f32),
        other => Err(format!("unsupported sample format {other}")),
    }
}

/// Turns device audio into the staging format: the first two channels (a
/// mono device on both), resampled to RATE, as s16le.
struct Converter {
    channels: usize,
    resamplers: Option<[Resampler; 2]>,
    left: Vec<f32>,
    right: Vec<f32>,
    out: [Vec<f32>; 2],
}

impl Converter {
    fn new(channels: usize, rate: u32) -> Self {
        Converter {
            channels: channels.max(1),
            resamplers: (rate != RATE).then(|| [Resampler::new(rate, RATE), Resampler::new(rate, RATE)]),
            left: Vec::new(),
            right: Vec::new(),
            out: [Vec::new(), Vec::new()],
        }
    }

    fn push(&mut self, samples: &[f32], bytes: &mut Vec<u8>) {
        self.left.clear();
        self.right.clear();
        for frame in samples.chunks_exact(self.channels) {
            self.left.push(frame[0]);
            self.right.push(*frame.get(1).unwrap_or(&frame[0]));
        }
        let (left, right) = match self.resamplers.as_mut() {
            Some([l, r]) => {
                self.out[0].clear();
                self.out[1].clear();
                l.process(&self.left, &mut self.out[0]);
                r.process(&self.right, &mut self.out[1]);
                (&self.out[0], &self.out[1])
            }
            None => (&self.left, &self.right),
        };
        for (l, r) in left.iter().zip(right) {
            for v in [l, r] {
                let s = (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                bytes.extend_from_slice(&s.to_le_bytes());
            }
        }
    }
}

fn handle_chunk(
    shared: &Mutex<Inner>,
    chunk: &[u8],
    waited: Duration,
    first: bool,
    chunks: u64,
    now: Instant,
) {
    let peak = chunk
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs())
        .max()
        .unwrap_or(0) as f32
        / 32768.0;
    let mut guard = shared.lock().unwrap();
    let inner = &mut *guard;
    inner.levels.pop_front();
    inner.levels.push_back(peak);
    inner.zero_chunks = if peak == 0.0 { inner.zero_chunks + 1 } else { 0 };
    if let Some(Recording { file, timeline }) = inner.recording.as_mut() {
        timeline.heard(waited, first);
        if inner.paused {
            return;
        }
        let missing = timeline.missing_before(now);
        let _ = write_silence(file, missing);
        timeline.wrote(missing + (CHUNK_BYTES / FRAME_BYTES) as u64);
        let _ = file.write_all(chunk);
        // So a crash loses at most a second: flush every second, and push it
        // to the disk itself every half minute in case the machine goes down too.
        if chunks.is_multiple_of(50) {
            let _ = file.flush();
        }
        if chunks.is_multiple_of(1500) {
            let _ = file.get_ref().sync_data();
        }
    }
}

/// Where a recording is: how much audio it should hold by now, against how
/// much it does. A track that falls behind, because the audio stopped coming
/// in for a while, catches up with silence, so it stays in step with the other
/// one. Time spent paused does not count.
struct Timeline {
    /// When the recording started, moved on by every pause.
    origin: Instant,
    paused_at: Option<Instant>,
    /// Frames in the file, silence included.
    frames: u64,
    /// Set when nothing came for a while, until the next chunk is written.
    quiet: bool,
}

impl Timeline {
    fn new(at: Instant) -> Self {
        Timeline {
            origin: at,
            paused_at: None,
            frames: 0,
            quiet: false,
        }
    }

    fn set_paused(&mut self, paused: bool, at: Instant) {
        if paused {
            self.paused_at.get_or_insert(at);
        } else if let Some(began) = self.paused_at.take() {
            self.origin += at.saturating_duration_since(began);
        }
    }

    /// Notes a chunk that came after a read that `waited`, the `first` of a
    /// stream, paused or not. Nothing came for a while when it is a new
    /// stream (the old one ended, or the device was not there yet), or when
    /// the stream was held for seconds, as by a Bluetooth headset switching to
    /// its headset profile. A read that returns at once finds audio that was
    /// only waiting for us, and the track catches up by itself.
    fn heard(&mut self, waited: Duration, first: bool) {
        self.quiet |= first || waited > STALL;
    }

    /// The frames of silence before a 20 ms chunk written at `at`: what the
    /// track is short of, after a quiet spell.
    fn missing_before(&self, at: Instant) -> u64 {
        if !self.quiet {
            return 0;
        }
        let due = frames_in(at.saturating_duration_since(self.origin));
        due.saturating_sub(self.frames + (CHUNK_BYTES / FRAME_BYTES) as u64)
    }

    fn wrote(&mut self, frames: u64) {
        self.frames += frames;
        self.quiet = false;
    }
}

fn frames_in(duration: Duration) -> u64 {
    (duration.as_micros() * u128::from(RATE) / 1_000_000) as u64
}

/// Writes `frames` of silence (s16le, RATE, CHANNELS).
fn write_silence(out: &mut impl Write, frames: u64) -> std::io::Result<()> {
    let zeros = [0u8; CHUNK_BYTES];
    let mut bytes = frames * FRAME_BYTES as u64;
    while bytes > 0 {
        let n = bytes.min(CHUNK_BYTES as u64) as usize;
        out.write_all(&zeros[..n])?;
        bytes -= n as u64;
    }
    Ok(())
}

/// Maps a linear peak to 0..1 on a -60 dB..0 dB scale.
pub fn to_meter(peak: f32) -> f64 {
    if peak <= 0.0 {
        return 0.0;
    }
    (1.0 - 20.0 * f64::from(peak).log10() / FLOOR_DB).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHUNK_FRAMES: u64 = (CHUNK_BYTES / FRAME_BYTES) as u64;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// A timeline that has had a chunk every 20 ms for `secs` seconds.
    fn running(t: Instant, secs: u64) -> Timeline {
        let mut timeline = Timeline::new(t);
        timeline.wrote(frames_in(Duration::from_secs(secs)));
        timeline
    }

    /// The silence written before a chunk that came at `at` after a read that
    /// `waited`, the `first` of a stream, and then the chunk.
    fn write(timeline: &mut Timeline, at: Instant, waited: Duration, first: bool) -> u64 {
        timeline.heard(waited, first);
        let missing = timeline.missing_before(at);
        timeline.wrote(missing + CHUNK_FRAMES);
        missing
    }

    #[test]
    fn audio_that_keeps_coming_needs_nothing() {
        let t = Instant::now();
        let mut timeline = running(t, 10);
        assert_eq!(write(&mut timeline, t + ms(10_020), ms(20), false), 0);
        // Ordinary scheduling: a read a little slower than a chunk.
        assert_eq!(write(&mut timeline, t + ms(10_220), ms(200), false), 0);
        // Late after a slow moment on our side, with the audio waiting in
        // the buffer: it is all still to come.
        assert_eq!(write(&mut timeline, t + ms(10_800), ms(0), false), 0);
    }

    #[test]
    fn a_hole_becomes_silence_once() {
        let t = Instant::now();
        let mut timeline = running(t, 10);
        // A stream held for five seconds, then a chunk.
        assert_eq!(
            write(&mut timeline, t + ms(15_020), ms(5_000), false),
            frames_in(ms(5_000))
        );
        assert_eq!(write(&mut timeline, t + ms(15_040), ms(20), false), 0);
        // A stream that ended and came back a second later.
        assert_eq!(
            write(&mut timeline, t + ms(16_060), ms(100), true),
            frames_in(ms(1_000))
        );
    }

    #[test]
    fn a_hole_across_a_pause_keeps_only_the_recorded_part() {
        let t = Instant::now();
        let mut timeline = running(t, 10);
        // The audio stops at 10 s, pause at 13 s, resume at 20 s, audio at 21 s.
        timeline.set_paused(true, t + ms(13_000));
        timeline.set_paused(false, t + ms(20_000));
        assert_eq!(
            write(&mut timeline, t + ms(21_020), ms(100), true),
            frames_in(ms(4_000))
        );
    }

    #[test]
    fn a_stream_back_during_a_pause_still_fills_the_hole() {
        let t = Instant::now();
        let mut timeline = running(t, 10);
        // The audio stops at 10 s, pause at 10.3 s, the new stream's first
        // chunk at 11 s, not written while paused, resume at 12 s.
        timeline.set_paused(true, t + ms(10_300));
        timeline.heard(ms(100), true);
        timeline.set_paused(false, t + ms(12_000));
        assert_eq!(
            write(&mut timeline, t + ms(12_020), ms(20), false),
            frames_in(ms(300))
        );
    }

    #[test]
    fn a_stall_from_before_the_start_counts_from_the_start() {
        let t = Instant::now();
        // The stream has been held for 8 s; the recording started at t.
        let mut timeline = Timeline::new(t);
        assert_eq!(
            write(&mut timeline, t + ms(3_020), ms(8_000), false),
            frames_in(ms(3_000))
        );
    }

    #[test]
    fn silence_is_zeros_in_whole_frames() {
        let mut out = Vec::new();
        write_silence(&mut out, 1234).unwrap();
        assert_eq!(out.len(), 1234 * FRAME_BYTES);
        assert!(out.iter().all(|&b| b == 0));
    }
}
