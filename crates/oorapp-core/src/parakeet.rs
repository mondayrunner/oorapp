//! NVIDIA Parakeet TDT 0.6B v3 as the speech model instead of whisper: 25
//! European languages (Dutch among them) found by itself, word times straight
//! from the decoder, and a transducer that stays quiet on music and silence
//! where whisper repeats a phrase it made up. Run locally through ONNX
//! Runtime (`parakeet-rs`), the int8 export from istupakov/parakeet-tdt-0.6b-v3-onnx,
//! pinned to one revision. Pick it with `model = "parakeet"` in config.toml.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use parakeet_rs::{ParakeetTDT, TimestampMode, Transcriber};

use crate::transcribe::{Abort, CANCELLED, Events, WHISPER_RATE, download, models_dir};

/// The names that pick this model in config.toml.
pub const NAMES: [&str; 3] = ["parakeet", "parakeet-v3", "parakeet-tdt-0.6b-v3"];
pub const NAME: &str = "parakeet-v3";
pub const SIZE_MB: u32 = 670;

const REPO: &str = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce";
const FILES: [(&str, u64); 4] = [
    ("encoder-model.int8.onnx", 600_000_000),
    ("decoder_joint-model.int8.onnx", 15_000_000),
    ("vocab.txt", 50_000),
    ("config.json", 10),
];

/// Long audio goes in pieces of at most this long, cut at the quietest moment
/// near the end: with longer pieces the int8 model skips whole sentences.
const MAX_CHUNK: usize = WHISPER_RATE * 20;
/// Where to look for that quiet moment: the last stretch of a chunk.
const CUT_WINDOW: usize = WHISPER_RATE * 6;

pub fn is_parakeet(name: &str) -> bool {
    NAMES.contains(&name.trim().to_ascii_lowercase().as_str())
}

fn dir() -> PathBuf {
    models_dir().join("parakeet-tdt-0.6b-v3")
}

/// The model folder, when every file is there.
pub fn find() -> Option<PathBuf> {
    let dir = dir();
    FILES
        .iter()
        .all(|(file, min)| std::fs::metadata(dir.join(file)).is_ok_and(|m| m.is_file() && m.len() >= *min))
        .then_some(dir)
}

pub fn ensure(events: &Events, abort: &Abort) -> Result<PathBuf, String> {
    let dir = dir();
    for (file, min_bytes) in FILES {
        let path = dir.join(file);
        if std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() >= min_bytes) {
            continue;
        }
        download(
            &format!("{REPO}/{file}"),
            &path,
            "Downloading model",
            min_bytes,
            events,
            abort,
        )?;
    }
    Ok(dir)
}

/// A word on the timeline of the samples handed in.
#[derive(Clone, Debug)]
pub struct Word {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    /// Which sentence it belongs to, counted over the whole input.
    pub sentence: usize,
}

pub struct Engine {
    model: ParakeetTDT,
}

impl Engine {
    pub fn load(events: &Events, abort: &Abort) -> Result<Self, String> {
        let dir = ensure(events, abort)?;
        if abort.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
        let config = parakeet_rs::ExecutionConfig::default().with_intra_threads(threads);
        let model = ParakeetTDT::from_pretrained(&dir, Some(config))
            .map_err(|e| format!("could not load the model {}: {e}", dir.display()))?;
        Ok(Engine { model })
    }

    /// Words with their times in `samples` (16 kHz mono). `on_chunk` hears how
    /// far it is (0..1) and the words of every piece as it is done.
    pub fn transcribe(
        &mut self,
        samples: &[f32],
        abort: &Abort,
        mut on_chunk: impl FnMut(f64, &[Word]),
    ) -> Result<Vec<Word>, String> {
        let mut words: Vec<Word> = Vec::new();
        let mut sentence = 0usize;
        for (start, end) in chunks(samples) {
            if abort.load(Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
            let piece = &samples[start..end];
            let first = words.len();
            if piece.iter().any(|v| v.abs() > 1e-4) {
                let result = self
                    .model
                    .transcribe_samples(piece.to_vec(), WHISPER_RATE as u32, 1, Some(TimestampMode::Words))
                    .map_err(|e| e.to_string())?;
                let offset_ms = (start * 1000 / WHISPER_RATE) as i64;
                for token in result.tokens {
                    let text = token.text.trim().to_owned();
                    if text.is_empty() {
                        continue;
                    }
                    let ends = text.ends_with(['.', '?', '!', '…']);
                    // Punctuation on its own belongs to the word before it.
                    if !text.chars().any(char::is_alphanumeric)
                        && words.len() > first
                        && let Some(last) = words.last_mut()
                    {
                        last.text.push_str(&text);
                        last.end_ms = last.end_ms.max(offset_ms + (token.end * 1000.0) as i64);
                        if ends {
                            sentence += 1;
                        }
                        continue;
                    }
                    words.push(Word {
                        text,
                        start_ms: offset_ms + (token.start * 1000.0) as i64,
                        end_ms: offset_ms + (token.end * 1000.0) as i64,
                        sentence,
                    });
                    if ends {
                        sentence += 1;
                    }
                }
                // A sentence never runs across a cut.
                if words.len() > first && words.last().is_some_and(|w| w.sentence == sentence) {
                    sentence += 1;
                }
            }
            on_chunk(end as f64 / samples.len().max(1) as f64, &words[first..]);
        }
        Ok(words)
    }
}

/// Splits `samples` into pieces of at most `MAX_CHUNK`, each cut at the
/// quietest 50 ms in the last `CUT_WINDOW` before the limit.
fn chunks(samples: &[f32]) -> Vec<(usize, usize)> {
    const FRAME: usize = WHISPER_RATE / 20;
    let mut out = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        let limit = start + MAX_CHUNK;
        if limit >= samples.len() {
            out.push((start, samples.len()));
            break;
        }
        let from = limit - CUT_WINDOW;
        let mut best = (f32::MAX, limit);
        let mut at = from;
        while at + FRAME <= limit {
            let energy: f32 = samples[at..at + FRAME].iter().map(|v| v * v).sum();
            if energy < best.0 {
                best = (energy, at + FRAME / 2);
            }
            at += FRAME;
        }
        out.push((start, best.1));
        start = best.1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_pick_parakeet() {
        assert!(is_parakeet("parakeet"));
        assert!(is_parakeet("Parakeet-V3"));
        assert!(!is_parakeet("large-v3-turbo"));
    }

    #[test]
    fn long_audio_is_cut_in_the_quiet() {
        // 100 s of tone with a silent half second at 17 s.
        let mut samples: Vec<f32> = (0..WHISPER_RATE * 100)
            .map(|i| (i as f32 * 0.05).sin() * 0.3)
            .collect();
        for v in &mut samples[WHISPER_RATE * 17..WHISPER_RATE * 17 + WHISPER_RATE / 2] {
            *v = 0.0;
        }
        let pieces = chunks(&samples);
        assert_eq!(pieces.first().unwrap().0, 0);
        assert_eq!(pieces.last().unwrap().1, samples.len());
        assert!(pieces.windows(2).all(|w| w[0].1 == w[1].0));
        assert!(pieces.iter().all(|(a, b)| b - a <= MAX_CHUNK));
        let cut = pieces[0].1;
        assert!((WHISPER_RATE * 17..WHISPER_RATE * 17 + WHISPER_RATE / 2).contains(&cut), "cut at {cut}");
    }
}
