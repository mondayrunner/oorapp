//! Audio in and out without ffmpeg: Ogg Opus written with libopus, any common
//! file read back (Opus through libopus, the rest through symphonia), a
//! streaming resampler, and the peak limiter the export runs every track
//! through. The upstream recorder shells out to ffmpeg for all of this; a
//! desktop app on macOS and Windows cannot count on it being there.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, Write};
use std::path::Path;

use crate::RATE;

// ---------------------------------------------------------------------------
// Writing Ogg Opus

/// 20 ms at 48 kHz, the frame size every encoder here uses.
const FRAME: usize = 960;
/// Largest Opus packet libopus produces.
const MAX_PACKET: usize = 4000;

/// Encodes interleaved f32 frames at 48 kHz into an Ogg Opus file.
pub struct OpusWriter {
    encoder: opus::Encoder,
    ogg: ogg::writing::PacketWriter<'static, BufWriter<File>>,
    serial: u32,
    channels: usize,
    pre_skip: u64,
    /// Samples per channel handed in so far.
    written: u64,
    pending: Vec<f32>,
    packet: Vec<u8>,
}

impl OpusWriter {
    pub fn create(path: &Path, channels: usize, bitrate: i32) -> Result<Self, String> {
        let layout = match channels {
            1 => opus::Channels::Mono,
            2 => opus::Channels::Stereo,
            n => return Err(format!("Opus here takes 1 or 2 channels, not {n}")),
        };
        let mut encoder = opus::Encoder::new(RATE, layout, opus::Application::Audio)
            .map_err(|e| format!("opus: {e}"))?;
        encoder
            .set_bitrate(opus::Bitrate::Bits(bitrate))
            .map_err(|e| format!("opus: {e}"))?;
        let pre_skip = encoder.get_lookahead().map_err(|e| format!("opus: {e}"))? as u64;
        let file = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut ogg = ogg::writing::PacketWriter::new(BufWriter::new(file));
        let serial = serial_for(path);

        // RFC 7845: the identification header and the comment header, each on
        // a page of its own, both with granule position 0.
        let mut head = b"OpusHead".to_vec();
        head.push(1); // version
        head.push(channels as u8);
        head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
        head.extend_from_slice(&RATE.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes()); // output gain
        head.push(0); // channel mapping family: mono or stereo
        let vendor = format!("{} {}", crate::APP_NAME, opus::version());
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor.as_bytes());
        tags.extend_from_slice(&0u32.to_le_bytes()); // no comments
        for header in [head, tags] {
            ogg.write_packet(header, serial, ogg::writing::PacketWriteEndInfo::EndPage, 0)
                .map_err(|e| e.to_string())?;
        }
        Ok(OpusWriter {
            encoder,
            ogg,
            serial,
            channels,
            pre_skip,
            written: 0,
            pending: Vec::with_capacity(FRAME * channels),
            packet: vec![0; MAX_PACKET],
        })
    }

    /// Adds interleaved samples, any amount.
    pub fn write(&mut self, samples: &[f32]) -> Result<(), String> {
        let frame = FRAME * self.channels;
        let mut rest = samples;
        while !rest.is_empty() {
            let take = (frame - self.pending.len()).min(rest.len());
            self.pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.pending.len() == frame {
                self.written += FRAME as u64;
                self.flush_frame(false)?;
            }
        }
        Ok(())
    }

    fn flush_frame(&mut self, last: bool) -> Result<(), String> {
        let len = self
            .encoder
            .encode_float(&self.pending, &mut self.packet)
            .map_err(|e| format!("opus: {e}"))?;
        self.pending.clear();
        let end = if last {
            ogg::writing::PacketWriteEndInfo::EndStream
        } else {
            ogg::writing::PacketWriteEndInfo::NormalPacket
        };
        // The granule position counts samples at 48 kHz including the
        // pre-skip; on the last page it says where the audio really ends, so
        // the padding of the final frame is cut off again on playback.
        let granule = self.pre_skip + self.written;
        self.ogg
            .write_packet(self.packet[..len].to_vec(), self.serial, end, granule)
            .map_err(|e| e.to_string())
    }

    pub fn finish(mut self) -> Result<(), String> {
        // Push the encoder's lookahead out with silence, then the padded last frame.
        let real = self.written + (self.pending.len() / self.channels) as u64;
        let tail = vec![0.0; self.pre_skip as usize * self.channels];
        let frame = FRAME * self.channels;
        let mut rest: &[f32] = &tail;
        loop {
            let take = (frame - self.pending.len()).min(rest.len());
            self.pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if rest.is_empty() {
                break;
            }
            self.written += FRAME as u64;
            self.flush_frame(false)?;
        }
        self.pending.resize(frame, 0.0);
        self.written = real;
        self.flush_frame(true)?;
        let mut inner = self.ogg.into_inner();
        inner.flush().map_err(|e| e.to_string())
    }
}

/// A stream serial that differs per file, as Ogg asks for.
fn serial_for(path: &Path) -> u32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    std::time::SystemTime::now().hash(&mut hasher);
    hasher.finish() as u32
}

// ---------------------------------------------------------------------------
// Reading

/// Decoded audio, interleaved.
pub struct Decoded {
    pub samples: Vec<f32>,
    pub channels: usize,
    pub rate: u32,
}

/// Decodes `path` to mono at `rate` Hz: what whisper and the speaker model take.
pub fn decode_mono(path: &Path, rate: u32) -> Result<Vec<f32>, String> {
    let mut out = Vec::new();
    let mut resampler: Option<Resampler> = None;
    let mut mono = Vec::new();
    decode_stream(path, &mut |chunk: &[f32], channels: usize, from: u32| {
        let r = resampler.get_or_insert_with(|| Resampler::new(from, rate));
        mono.clear();
        mono.extend(
            chunk
                .chunks_exact(channels)
                .map(|f| f.iter().sum::<f32>() / channels as f32),
        );
        r.process(&mono, &mut out);
    })?;
    if let Some(r) = resampler.as_mut() {
        r.finish(&mut out);
    }
    Ok(out)
}

/// Decodes `path` into a raw staging track (s16le, RATE, CHANNELS), mixed
/// to mono and the same on both channels, streaming, so a long file never
/// sits in memory whole. Returns the frames written.
pub fn decode_to_raw(path: &Path, raw: &Path) -> Result<u64, String> {
    let mut out = BufWriter::new(File::create(raw).map_err(|e| format!("{}: {e}", raw.display()))?);
    let mut resampler: Option<Resampler> = None;
    let (mut mono, mut resampled) = (Vec::new(), Vec::new());
    let mut frames = 0u64;
    let mut failed = None;
    let write = |samples: &[f32], out: &mut BufWriter<File>| {
        let mut bytes = Vec::with_capacity(samples.len() * 4);
        for v in samples {
            let s = (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            for _ in 0..crate::CHANNELS {
                bytes.extend_from_slice(&s.to_le_bytes());
            }
        }
        out.write_all(&bytes)
    };
    decode_stream(path, &mut |chunk: &[f32], channels: usize, from: u32| {
        let r = resampler.get_or_insert_with(|| Resampler::new(from, RATE));
        mono.clear();
        mono.extend(
            chunk
                .chunks_exact(channels)
                .map(|f| f.iter().sum::<f32>() / channels as f32),
        );
        resampled.clear();
        r.process(&mono, &mut resampled);
        frames += resampled.len() as u64;
        if let Err(e) = write(&resampled, &mut out) {
            failed.get_or_insert(e.to_string());
        }
    })?;
    if let Some(r) = resampler.as_mut() {
        resampled.clear();
        r.finish(&mut resampled);
        frames += resampled.len() as u64;
        write(&resampled, &mut out).map_err(|e| e.to_string())?;
    }
    if let Some(e) = failed {
        return Err(e);
    }
    out.flush().map_err(|e| e.to_string())?;
    Ok(frames)
}

/// Decodes all of `path`, keeping its channels and rate.
pub fn decode(path: &Path) -> Result<Decoded, String> {
    let mut decoded = Decoded {
        samples: Vec::new(),
        channels: 1,
        rate: RATE,
    };
    decode_stream(path, &mut |chunk, channels, rate| {
        decoded.channels = channels;
        decoded.rate = rate;
        decoded.samples.extend_from_slice(chunk);
    })?;
    Ok(decoded)
}

/// Length of an audio file in seconds, 0.0 when it cannot be read.
pub fn duration_secs(path: &Path) -> f64 {
    let (mut frames, mut rate) = (0u64, RATE);
    let ok = decode_stream(path, &mut |chunk, channels, r| {
        frames += (chunk.len() / channels) as u64;
        rate = r;
    });
    if ok.is_err() || rate == 0 {
        0.0
    } else {
        frames as f64 / f64::from(rate)
    }
}

/// Hands the decoded audio of `path` to `sink` in chunks: interleaved
/// samples, the channel count and the sample rate.
fn decode_stream(path: &Path, sink: &mut dyn FnMut(&[f32], usize, u32)) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut magic = [0u8; 64];
    let n = file.read(&mut magic).unwrap_or(0);
    file.rewind().map_err(|e| e.to_string())?;
    // Ogg Opus: the first page holds "OpusHead" at byte 28.
    if n >= 36 && &magic[..4] == b"OggS" && &magic[28..36] == b"OpusHead" {
        return decode_ogg_opus(file, sink).map_err(|e| format!("{}: {e}", path.display()));
    }
    decode_symphonia(file, path, sink)
}

fn decode_ogg_opus(file: File, sink: &mut dyn FnMut(&[f32], usize, u32)) -> Result<(), String> {
    let mut reader = ogg::reading::PacketReader::new(BufReader::new(file));
    let head = reader
        .read_packet()
        .map_err(|e| e.to_string())?
        .ok_or("empty Ogg file")?;
    if head.data.len() < 19 || &head.data[..8] != b"OpusHead" {
        return Err("not an Opus stream".into());
    }
    let channels = usize::from(head.data[9]);
    let pre_skip = u64::from(u16::from_le_bytes([head.data[10], head.data[11]]));
    let layout = match channels {
        1 => opus::Channels::Mono,
        2 => opus::Channels::Stereo,
        n => return Err(format!("{n}-channel Opus is not supported")),
    };
    let mut decoder = opus::Decoder::new(RATE, layout).map_err(|e| e.to_string())?;
    let _tags = reader.read_packet().map_err(|e| e.to_string())?;
    // 120 ms, the longest Opus packet.
    let mut pcm = vec![0f32; 5760 * channels];
    let (mut decoded, mut skip) = (0u64, pre_skip);
    while let Some(packet) = reader.read_packet().map_err(|e| e.to_string())? {
        let frames = decoder
            .decode_float(&packet.data, &mut pcm, false)
            .map_err(|e| e.to_string())? as u64;
        let mut start = 0u64;
        if skip > 0 {
            start = skip.min(frames);
            skip -= start;
        }
        let mut end = frames;
        // The last page's granule position marks where the audio ends.
        if packet.last_in_stream() {
            let total = packet.absgp_page().saturating_sub(pre_skip);
            end = end.min(start + total.saturating_sub(decoded));
        }
        if end > start {
            decoded += end - start;
            sink(
                &pcm[start as usize * channels..end as usize * channels],
                channels,
                RATE,
            );
        }
    }
    Ok(())
}

fn decode_symphonia(
    file: File,
    path: &Path,
    sink: &mut dyn FnMut(&[f32], usize, u32),
) -> Result<(), String> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let cant = |e: &dyn std::fmt::Display| format!("could not read {}: {e}", path.display());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| cant(&e))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| cant(&"no audio track"))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| cant(&"no audio track"))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| cant(&e))?;
    let mut samples: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(cant(&e)),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let channels = buf.spec().channels().count().max(1);
                let rate = buf.spec().rate();
                samples.clear();
                buf.copy_to_vec_interleaved(&mut samples);
                sink(&samples, channels, rate);
            }
            Err(Error::DecodeError(_)) => {}
            Err(e) => return Err(cant(&e)),
        }
    }
    Ok(())
}

/// Reads a raw staging track (s16le, interleaved) in frames.
pub struct RawReader {
    reader: BufReader<File>,
    channels: usize,
    buf: Vec<u8>,
}

impl RawReader {
    pub fn open(path: &Path, channels: usize) -> std::io::Result<Self> {
        Ok(RawReader {
            reader: BufReader::with_capacity(1 << 16, File::open(path)?),
            channels,
            buf: Vec::new(),
        })
    }

    /// Up to `frames` frames as f32 into `out`; the number read, 0 at the end.
    pub fn read(&mut self, frames: usize, out: &mut Vec<f32>) -> std::io::Result<usize> {
        let want = frames * self.channels * 2;
        self.buf.resize(want, 0);
        let mut got = 0;
        while got < want {
            match self.reader.read(&mut self.buf[got..want])? {
                0 => break,
                n => got += n,
            }
        }
        let frame_bytes = self.channels * 2;
        let whole = got / frame_bytes;
        out.clear();
        out.extend(
            self.buf[..whole * frame_bytes]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0),
        );
        Ok(whole)
    }
}

// ---------------------------------------------------------------------------
// Resampling

/// Taps on either side of the windowed-sinc kernel, at the lower rate.
const HALF_TAPS: usize = 32;

/// Streaming mono resampler: a Blackman-windowed sinc evaluated per output
/// sample, cut off just below the lower of the two Nyquist frequencies.
/// Plenty for speech recognition and waveforms, and it only keeps a short
/// history.
pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Kernel half-width in input samples.
    reach: usize,
    /// Input not yet used up, starting `reach` samples before `pos`.
    buf: Vec<f32>,
    /// Where the next output sample falls, as an index into `buf`.
    pos: f64,
    taken: u64,
    emitted: u64,
    passthrough: bool,
    /// The kernel sampled `TABLE_RES` times per input sample over
    /// `-reach..=reach`, interpolated linearly: no trigonometry per tap.
    table: Vec<f32>,
}

const TABLE_RES: usize = 256;

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        let step = f64::from(from) / f64::from(to);
        let scale = (1.0 / step).min(1.0);
        let reach = (HALF_TAPS as f64 / scale).ceil() as usize;
        let cutoff = 0.5 * 0.97 * scale;
        let table = (0..=2 * reach * TABLE_RES)
            .map(|j| kernel(j as f64 / TABLE_RES as f64 - reach as f64, reach as f64, cutoff) as f32)
            .collect();
        Resampler {
            step,
            reach,
            buf: vec![0.0; reach],
            pos: reach as f64,
            taken: 0,
            emitted: 0,
            passthrough: from == to,
            table,
        }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.passthrough {
            out.extend_from_slice(input);
            return;
        }
        self.taken += input.len() as u64;
        self.buf.extend_from_slice(input);
        self.drain(out, u64::MAX);
    }

    /// Flushes what is left once the input has ended.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        if self.passthrough {
            return;
        }
        self.buf.extend(std::iter::repeat_n(0.0, self.reach + 1));
        let total = (self.taken as f64 / self.step).round() as u64;
        self.drain(out, total);
    }

    fn drain(&mut self, out: &mut Vec<f32>, until: u64) {
        let reach = self.reach as isize;
        while self.emitted < until && (self.pos.floor() as usize) + self.reach < self.buf.len() {
            let centre = self.pos.floor() as isize;
            let frac = self.pos - centre as f64;
            let (mut acc, mut norm) = (0.0f32, 0.0f32);
            // Table position of tap k: (k - frac + reach) * TABLE_RES.
            let base = (reach as f64 - frac) * TABLE_RES as f64;
            let last = self.table.len() - 1;
            for k in -reach..=reach {
                let t = base + (k * TABLE_RES as isize) as f64;
                if t < 0.0 {
                    continue;
                }
                let (j, f) = (t as usize, (t - t.floor()) as f32);
                if j >= last {
                    continue;
                }
                let w = self.table[j] + (self.table[j + 1] - self.table[j]) * f;
                acc += self.buf[(centre + k) as usize] * w;
                norm += w;
            }
            out.push(if norm.abs() > 1e-6 { acc / norm } else { 0.0 });
            self.emitted += 1;
            self.pos += self.step;
        }
        // Keep `reach` samples of history before the next position.
        let keep_from = (self.pos.floor() as usize).saturating_sub(self.reach).min(self.buf.len());
        if keep_from > 0 {
            self.buf.drain(..keep_from);
            self.pos -= keep_from as f64;
        }
    }
}

/// A Blackman-windowed sinc at `x` input samples from the centre, `half`
/// samples wide on either side, cut off at `cutoff` cycles per sample.
fn kernel(x: f64, half: f64, cutoff: f64) -> f64 {
    if x.abs() >= half {
        return 0.0;
    }
    let sinc = if x == 0.0 {
        2.0 * cutoff
    } else {
        (std::f64::consts::TAU * cutoff * x).sin() / (std::f64::consts::PI * x)
    };
    let n = (x + half) / (2.0 * half);
    let window = 0.42 - 0.5 * (std::f64::consts::TAU * n).cos()
        + 0.08 * (2.0 * std::f64::consts::TAU * n).cos();
    sinc * window
}

// ---------------------------------------------------------------------------
// Limiting

/// A lookahead peak limiter that keeps every sample under `ceiling`, linked
/// over the channels, in the spirit of ffmpeg's `alimiter`: the gain dips
/// smoothly a few milliseconds before a peak and recovers over 50 ms. The
/// output is `delay()` frames behind the input.
pub struct Limiter {
    channels: usize,
    ceiling: f32,
    lookahead: usize,
    release_step: f32,
    /// Input frames waiting to come out.
    delay_line: std::collections::VecDeque<f32>,
    /// Sliding minimum of `needed`, as a monotonic queue of (index, gain).
    minimum: std::collections::VecDeque<(u64, f32)>,
    /// The last `lookahead` minima, for the smoothing average.
    smooth: std::collections::VecDeque<f32>,
    smooth_sum: f64,
    index: u64,
    gain: f32,
}

impl Limiter {
    pub fn new(channels: usize, ceiling: f32) -> Self {
        let lookahead = (RATE as usize) / 200; // 5 ms
        Limiter {
            channels,
            ceiling,
            lookahead,
            release_step: 1.0 / (0.05 * RATE as f32),
            delay_line: Default::default(),
            minimum: Default::default(),
            smooth: Default::default(),
            smooth_sum: 0.0,
            index: 0,
            gain: 1.0,
        }
    }

    /// Frames between a sample going in and coming out.
    pub fn delay(&self) -> usize {
        self.lookahead * 3 / 2
    }

    /// Feeds interleaved `input` and appends what is ready to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for frame in input.chunks_exact(self.channels) {
            let peak = frame.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let needed = if peak > self.ceiling { self.ceiling / peak } else { 1.0 };
            self.delay_line.extend(frame);
            self.push(needed, out);
        }
    }

    /// Pushes the delayed tail out.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        let silence = vec![0.0; self.channels];
        for _ in 0..self.delay() {
            self.delay_line.extend(&silence);
            self.push(1.0, out);
        }
    }

    fn push(&mut self, needed: f32, out: &mut Vec<f32>) {
        let window = 2 * self.lookahead as u64;
        // Sliding minimum over the last 2*lookahead+1 frames.
        while self.minimum.back().is_some_and(|&(_, g)| g >= needed) {
            self.minimum.pop_back();
        }
        self.minimum.push_back((self.index, needed));
        while self
            .minimum
            .front()
            .is_some_and(|&(i, _)| i + window < self.index)
        {
            self.minimum.pop_front();
        }
        let min = self.minimum.front().map_or(1.0, |&(_, g)| g);
        // Average of the last `lookahead` minima: a ramp instead of a step,
        // and never above what the delayed frame needs.
        self.smooth.push_back(min);
        self.smooth_sum += f64::from(min);
        if self.smooth.len() > self.lookahead {
            self.smooth_sum -= f64::from(self.smooth.pop_front().unwrap_or(1.0));
        }
        let target = (self.smooth_sum / self.smooth.len() as f64) as f32;
        self.gain = target.min(self.gain + self.release_step);
        self.index += 1;
        if self.delay_line.len() / self.channels > self.delay() {
            for _ in 0..self.channels {
                let v = self.delay_line.pop_front().unwrap_or(0.0);
                out.push((v * self.gain).clamp(-self.ceiling, self.ceiling));
            }
        }
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f64, rate: u32, secs: f64, amp: f32) -> Vec<f32> {
        (0..(f64::from(rate) * secs) as usize)
            .map(|i| amp * (i as f64 * freq * std::f64::consts::TAU / f64::from(rate)).sin() as f32)
            .collect()
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt()
    }

    #[test]
    fn opus_round_trip_keeps_length_and_level() {
        let dir = std::env::temp_dir().join(format!("oorapp-codec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.ogg");
        let input = tone(440.0, RATE, 2.5, 0.5);
        let mut writer = OpusWriter::create(&path, 1, 64_000).unwrap();
        writer.write(&input).unwrap();
        writer.finish().unwrap();
        let back = decode(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(back.channels, 1);
        assert_eq!(back.samples.len(), input.len());
        assert!((rms(&back.samples) - rms(&input)).abs() < 0.02);
    }

    #[test]
    fn resampling_keeps_duration_and_level() {
        let input = tone(300.0, 44_100, 3.0, 0.5);
        let mut out = Vec::new();
        let mut r = Resampler::new(44_100, 16_000);
        for chunk in input.chunks(1000) {
            r.process(chunk, &mut out);
        }
        r.finish(&mut out);
        let expected = 3 * 16_000;
        assert!((out.len() as i64 - expected).abs() <= 2, "{} samples", out.len());
        assert!((rms(&out[1000..out.len() - 1000]) - 0.3536).abs() < 0.01);
    }

    #[test]
    fn limiter_keeps_peaks_under_the_ceiling() {
        let mut input = tone(200.0, RATE, 1.0, 0.3);
        for v in &mut input[20_000..20_500] {
            *v *= 4.0; // a burst to 1.2
        }
        let mut limiter = Limiter::new(1, 0.9);
        let mut out = Vec::new();
        limiter.process(&input, &mut out);
        limiter.finish(&mut out);
        assert_eq!(out.len(), input.len());
        assert!(out.iter().all(|v| v.abs() <= 0.9 + 1e-6));
        // Away from the burst nothing changes.
        assert!((out[5000] - input[5000]).abs() < 1e-6);
    }
}
