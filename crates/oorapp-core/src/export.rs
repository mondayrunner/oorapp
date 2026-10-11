//! Turns the two raw tracks into the audio file(s) the user asked for.

use std::fs::OpenOptions;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use crate::codec::{Limiter, OpusWriter, RawReader};
use crate::{CHANNELS, RATE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Mono,
    Stereo,
    Separate,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Mono, Format::Stereo, Format::Separate];

    pub fn key(self) -> &'static str {
        match self {
            Format::Mono => "mono",
            Format::Stereo => "stereo",
            Format::Separate => "separate",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Mono => "Mono",
            Format::Stereo => "Stereo (mic left, computer right)",
            Format::Separate => "Separate files",
        }
    }

    /// For the one-line summary on the done page.
    pub fn short_label(self) -> &'static str {
        match self {
            Format::Mono => "Mono",
            Format::Stereo => "Stereo",
            Format::Separate => "Separate files",
        }
    }

    pub fn from_key(key: &str) -> Format {
        Format::ALL
            .into_iter()
            .find(|f| f.key() == key)
            .unwrap_or(Format::Mono)
    }
}

/// Appends silence so both tracks are equally long and stay aligned at the end.
fn pad_to_same_length(paths: [&Path; 2]) -> std::io::Result<()> {
    let sizes = [
        std::fs::metadata(paths[0])?.len(),
        std::fs::metadata(paths[1])?.len(),
    ];
    let length = sizes[0].max(sizes[1]);
    for (path, size) in paths.into_iter().zip(sizes) {
        if size < length {
            let mut file = OpenOptions::new().append(true).open(path)?;
            file.write_all(&vec![0u8; (length - size) as usize])?;
        }
    }
    Ok(())
}

/// Speech level both tracks are brought to: the 95th percentile of 50 ms RMS
/// frames, about -18 dBFS. Loud enough to listen to, with room for peaks.
const TARGET_LEVEL: f64 = 0.125;
/// Below this (about -55 dBFS) a track is silence or hiss and is left alone,
/// so the noise floor is never pumped up to speech level.
const SILENT_LEVEL: f64 = 0.0018;
const MAX_BOOST_DB: f64 = 24.0;
const MAX_CUT_DB: f64 = -12.0;

/// The fixed gain in dB that brings the speech in a raw track to `TARGET_LEVEL`.
/// One gain for the whole track keeps its dynamics; the limiter after it
/// catches the peaks.
pub fn speech_gain_db(raw: &Path) -> f64 {
    let Ok(file) = std::fs::File::open(raw) else {
        return 0.0;
    };
    let mut reader = BufReader::new(file);
    let frame_bytes = (RATE / 20 * CHANNELS * 2) as usize; // 50 ms
    let mut buf = vec![0u8; frame_bytes];
    let mut levels = Vec::new();
    while reader.read_exact(&mut buf).is_ok() {
        let sum: f64 = buf
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f64::from(i16::from_le_bytes(*b)) / 32768.0)
            .map(|v| v * v)
            .sum();
        levels.push((sum / (frame_bytes / 2) as f64).sqrt());
    }
    if levels.is_empty() {
        return 0.0;
    }
    levels.sort_by(f64::total_cmp);
    let p95 = levels[(levels.len() - 1) * 95 / 100];
    if p95 < SILENT_LEVEL {
        return 0.0;
    }
    (20.0 * (TARGET_LEVEL / p95).log10()).clamp(MAX_CUT_DB, MAX_BOOST_DB)
}

/// Encodes `mic_raw` and `system_raw` into `out`. True when every file was written.
/// Each track is levelled on its own first, so a quiet side is as easy to hear
/// as a loud one.
pub fn export_audio(mic_raw: &Path, system_raw: &Path, out: &Path, format: Format) -> bool {
    for path in [mic_raw, system_raw] {
        if OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .is_err()
        {
            return false;
        }
    }
    if pad_to_same_length([mic_raw, system_raw]).is_err() {
        return false;
    }
    if std::fs::metadata(mic_raw).map(|m| m.len()).unwrap_or(0) == 0 {
        return true; // nothing was recorded
    }

    let mic_gain = db_to_gain(speech_gain_db(mic_raw));
    let system_gain = db_to_gain(speech_gain_db(system_raw));
    let mono = |f: &[f32]| (f[0] + f[1]) * 0.5;
    let result = match format {
        Format::Separate => encode(
            &[mic_raw],
            &out.join("mic.ogg"),
            1,
            64_000,
            Some(LIMIT),
            |frames, dst| dst.push(mono(frames[0]) * mic_gain),
        )
        .and_then(|()| {
            encode(
                &[system_raw],
                &out.join("computer.ogg"),
                2,
                96_000,
                Some(LIMIT),
                |frames, dst| dst.extend(frames[0].iter().map(|v| v * system_gain)),
            )
        }),
        Format::Stereo => encode(
            &[mic_raw, system_raw],
            &out.join("audio.ogg"),
            2,
            96_000,
            Some(LIMIT),
            |frames, dst| {
                dst.push(mono(frames[0]) * mic_gain);
                dst.push(mono(frames[1]) * system_gain);
            },
        ),
        Format::Mono => encode(
            &[mic_raw, system_raw],
            &out.join("audio.ogg"),
            1,
            64_000,
            Some(LIMIT),
            |frames, dst| dst.push(mono(frames[0]) * mic_gain + mono(frames[1]) * system_gain),
        ),
    };
    if let Err(e) = &result {
        eprintln!("{}: could not write the audio: {e}", crate::APP_NAME);
    }
    result.is_ok()
}

/// The limiter's ceiling, as ffmpeg's `alimiter=limit=0.9` upstream.
const LIMIT: f32 = 0.9;

fn db_to_gain(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

/// Reads the raw `inputs` in step, lets `mix` turn one frame of each into an
/// output frame, limits that when asked and writes it as Ogg Opus.
fn encode(
    inputs: &[&Path],
    target: &Path,
    channels: usize,
    bitrate: i32,
    limit: Option<f32>,
    mix: impl Fn(&[&[f32]], &mut Vec<f32>),
) -> Result<(), String> {
    const CHUNK: usize = 4800; // 100 ms
    let ch = CHANNELS as usize;
    let mut readers = inputs
        .iter()
        .map(|p| RawReader::open(p, ch).map_err(|e| format!("{}: {e}", p.display())))
        .collect::<Result<Vec<_>, _>>()?;
    let mut writer = OpusWriter::create(target, channels, bitrate)?;
    let mut limiter = limit.map(|ceiling| Limiter::new(channels, ceiling));
    let mut bufs = vec![Vec::new(); readers.len()];
    let (mut mixed, mut limited) = (Vec::new(), Vec::new());
    loop {
        let mut frames = 0;
        for (reader, buf) in readers.iter_mut().zip(bufs.iter_mut()) {
            frames = frames.max(reader.read(CHUNK, buf).map_err(|e| e.to_string())?);
        }
        if frames == 0 {
            break;
        }
        for buf in &mut bufs {
            buf.resize(frames * ch, 0.0);
        }
        mixed.clear();
        let mut frame_refs: Vec<&[f32]> = Vec::with_capacity(bufs.len());
        for i in 0..frames {
            frame_refs.clear();
            frame_refs.extend(bufs.iter().map(|b| &b[i * ch..(i + 1) * ch]));
            mix(&frame_refs, &mut mixed);
        }
        match limiter.as_mut() {
            Some(limiter) => {
                limited.clear();
                limiter.process(&mixed, &mut limited);
                writer.write(&limited)?;
            }
            None => writer.write(&mixed)?,
        }
    }
    if let Some(limiter) = limiter.as_mut() {
        limited.clear();
        limiter.finish(&mut limited);
        writer.write(&limited)?;
    }
    writer.finish()
}

/// The hidden directory in a meeting folder that keeps both tracks, so the
/// meeting can be transcribed again with You and Remote apart, whatever format
/// the user picked.
pub const TRACKS_DIR: &str = ".tracks";

pub fn tracks(meeting_dir: &Path) -> (PathBuf, PathBuf) {
    let dir = meeting_dir.join(TRACKS_DIR);
    (dir.join("mic.ogg"), dir.join("computer.ogg"))
}

/// Writes both raw tracks as mono Opus into `meeting_dir/.tracks`.
pub fn export_tracks(mic_raw: &Path, system_raw: &Path, meeting_dir: &Path) -> bool {
    let (mic, computer) = tracks(meeting_dir);
    if std::fs::create_dir_all(meeting_dir.join(TRACKS_DIR)).is_err() {
        return false;
    }
    [(mic_raw, mic), (system_raw, computer)]
        .iter()
        .all(|(raw, target)| export_mono(raw, target, 48_000))
}

/// A raw track as mono Opus at `bitrate`, as it is: no levelling.
pub fn export_mono(raw: &Path, target: &Path, bitrate: i32) -> bool {
    encode(&[raw], target, 1, bitrate, None, |frames, dst| {
        dst.push((frames[0][0] + frames[0][1]) * 0.5)
    })
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A raw track (s16le, RATE, CHANNELS) of a 440 Hz tone at `amplitude`.
    fn tone(path: &Path, amplitude: f64, secs: u32) {
        let mut bytes = Vec::new();
        for i in 0..RATE * secs {
            let v = (amplitude
                * (i as f64 * 440.0 * std::f64::consts::TAU / RATE as f64).sin()
                * 32767.0) as i16;
            for _ in 0..CHANNELS {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        std::fs::write(path, bytes).unwrap();
    }

    /// Mean level in dB of a decoded file, as ffmpeg's `volumedetect` reports it.
    fn mean_db(path: &Path) -> f64 {
        let decoded = crate::codec::decode(path).unwrap();
        let power: f64 = decoded.samples.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>()
            / decoded.samples.len() as f64;
        10.0 * power.log10()
    }

    #[test]
    fn quiet_and_loud_tracks_end_up_at_the_same_level() {
        let dir = std::env::temp_dir().join(format!("omr-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (mic, system) = (dir.join("mic.raw"), dir.join("system.raw"));
        tone(&mic, 0.5, 3); // loud
        tone(&system, 0.01, 3); // 34 dB quieter
        assert!(export_audio(&mic, &system, &dir, Format::Separate));
        let (loud, quiet) = (
            mean_db(&dir.join("mic.ogg")),
            mean_db(&dir.join("computer.ogg")),
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            (loud - quiet).abs() < 1.5,
            "mic {loud} dB, computer {quiet} dB"
        );
    }

    #[test]
    fn silence_is_not_boosted() {
        let dir = std::env::temp_dir().join(format!("omr-silence-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let raw = dir.join("hiss.raw");
        tone(&raw, 0.0005, 2); // about -66 dBFS
        let gain = speech_gain_db(&raw);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(gain, 0.0);
    }
}
