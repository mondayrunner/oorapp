//! What happens to a meeting between Stop and the done page, without a GUI:
//! saving the audio, transcribing it, naming the speakers, the chapters, an
//! import, editing the transcript and renaming the meeting. The upstream
//! recorder keeps this in its GTK window; here the app and the command line
//! share it.

use std::path::{Path, PathBuf};

use crate::chapters::{self, Chapter};
use crate::export::{self, Format};
use crate::meeting::{self, Manifest};
use crate::transcribe::{self, Abort, Events};
use crate::{CHANNELS, RATE};

/// Where the audio to transcribe comes from.
#[derive(Clone, Debug)]
pub enum Tracks {
    /// The raw staging folder of a recording that just stopped.
    Raw(PathBuf),
    /// The `.tracks` kept in a meeting folder, for transcribing again.
    Kept(PathBuf),
    /// One imported file, with the number of speakers asked for.
    Single(PathBuf, Option<usize>),
}

/// The imported source audio, kept for transcribing again.
pub fn source_track(meeting_dir: &Path) -> PathBuf {
    meeting_dir.join(export::TRACKS_DIR).join("source.ogg")
}

/// What to transcribe again for a meeting folder: the imported source, or the
/// two kept tracks.
pub fn kept_tracks(dir: &Path, manifest: &Manifest) -> Tracks {
    if manifest.imported.is_some() {
        Tracks::Single(source_track(dir), manifest.speaker_count)
    } else {
        Tracks::Kept(dir.to_owned())
    }
}

/// `~/Documents/Meetings/202610111430 Weekly sync`.
pub fn output_dir(started_at: i64, title: &str) -> PathBuf {
    let stamp = jiff::Timestamp::from_second(started_at)
        .map(|t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y%m%d%H%M")
                .to_string()
        })
        .unwrap_or_default();
    crate::paths::meetings_dir().join(format!("{stamp} {}", crate::safe_name(title)))
}

/// A free folder for `title`: `Weekly`, else `Weekly 2`, `Weekly 3` and so on.
pub fn free_output_dir(started_at: i64, title: &str) -> PathBuf {
    let mut out = output_dir(started_at, title);
    let mut n = 2;
    while out.exists() {
        out = output_dir(started_at, &format!("{title} {n}"));
        n += 1;
    }
    out
}

/// Whether `folder` is the folder of a meeting that would get `expected`:
/// "202609241400 Weekly 2" still belongs to "Weekly", numbered when the name
/// was taken.
pub fn folder_is_for(folder: &Path, expected: &Path) -> bool {
    let (Some(name), Some(want)) = (
        folder.file_name().and_then(|n| n.to_str()),
        expected.file_name().and_then(|n| n.to_str()),
    ) else {
        return folder == expected;
    };
    folder.parent() == expected.parent()
        && name.strip_prefix(want).is_some_and(|rest| {
            rest.is_empty()
                || rest
                    .strip_prefix(' ')
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
}

pub fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// "Meeting 14:30", the name of a meeting nobody named.
pub fn default_title() -> String {
    format!(
        "Meeting {}",
        jiff::Zoned::now().strftime("%H:%M")
    )
}

/// Every meeting in the meetings folder, newest first. Folders from before
/// manifests existed are read from their name; anything else is skipped.
pub fn list_meetings() -> Vec<(PathBuf, Manifest)> {
    list_meetings_in(&crate::paths::meetings_dir())
}

pub fn list_meetings_in(root: &Path) -> Vec<(PathBuf, Manifest)> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, Manifest)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|dir| meeting::open(&dir))
        .collect();
    found.sort_by(|a, b| b.1.started_at.cmp(&a.1.started_at).then_with(|| b.0.cmp(&a.0)));
    found
}

/// One line of the history: a meeting, an imported file or a quick note.
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub path: PathBuf,
    pub title: String,
    pub started_at: i64,
    pub duration_secs: i64,
    /// "note", "file" or "meeting".
    pub kind: &'static str,
    /// A note's whole text; for a meeting the start of the transcript.
    pub text: String,
    pub words: usize,
    pub transcribed: bool,
    pub target_app: Option<String>,
}

/// Characters of a meeting's transcript shown in the history.
const PREVIEW_CHARS: usize = 280;

pub fn history() -> Vec<HistoryEntry> {
    list_meetings()
        .into_iter()
        .map(|(dir, manifest)| history_entry(&dir, &manifest))
        .collect()
}

pub fn history_entry(dir: &Path, manifest: &Manifest) -> HistoryEntry {
    let markdown = std::fs::read_to_string(dir.join("transcript.md")).ok();
    let view = markdown.as_deref().map(View::parse).unwrap_or_default();
    let kind = if manifest.kind.as_deref() == Some("note") {
        "note"
    } else if manifest.imported.is_some() {
        "file"
    } else {
        "meeting"
    };
    let words = view
        .paragraphs
        .iter()
        .map(|p| p.text.split_whitespace().count())
        .sum();
    let text = if kind == "note" {
        view.paragraphs.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(" ")
    } else {
        let mut preview = String::new();
        for p in &view.paragraphs {
            if !preview.is_empty() {
                preview.push(' ');
            }
            preview.push_str(&format!("{}: {}", p.speaker, p.text));
            if preview.chars().count() > PREVIEW_CHARS {
                break;
            }
        }
        if preview.chars().count() > PREVIEW_CHARS {
            preview = preview.chars().take(PREVIEW_CHARS).collect::<String>().trim_end().to_owned() + "…";
        }
        preview
    };
    HistoryEntry {
        path: dir.to_owned(),
        title: manifest.title.clone(),
        started_at: manifest.started_at,
        duration_secs: manifest.duration_secs,
        kind,
        text,
        words,
        transcribed: markdown.is_some(),
        target_app: manifest.target_app.clone(),
    }
}

/// Moves a meeting folder to the trash, never further: it can be put back.
pub fn trash_meeting(dir: &Path) -> Result<(), String> {
    let root = crate::paths::meetings_dir();
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if dir.parent() != Some(root.as_path()) || meeting::open(&dir).is_none() {
        return Err("That is not a meeting folder".into());
    }
    trash::delete(&dir).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Recording

/// What is known about a recording in progress, next to its audio, so it can
/// be finished after a crash.
#[derive(Clone, Debug)]
pub struct RecordingNote {
    pub title: String,
    pub started_at: i64,
    pub format: Format,
    pub language: String,
}

/// A new staging folder for a recording that starts now.
pub fn new_staging(started_at: i64) -> std::io::Result<PathBuf> {
    let dir = crate::paths::recordings_dir().join(started_at.to_string());
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn mic_raw(staging: &Path) -> PathBuf {
    staging.join("mic.raw")
}

pub fn system_raw(staging: &Path) -> PathBuf {
    staging.join("system.raw")
}

pub fn write_recording_note(staging: &Path, note: &RecordingNote) {
    let json = serde_json::json!({
        "title": note.title,
        "started_at": note.started_at,
        "format": note.format.key(),
        "language": note.language,
    });
    let _ = std::fs::write(staging.join("recording.json"), json.to_string());
}

pub fn read_recording_note(staging: &Path) -> Option<RecordingNote> {
    let text = std::fs::read_to_string(staging.join("recording.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(RecordingNote {
        title: value["title"]
            .as_str()
            .filter(|t| !t.is_empty())?
            .to_owned(),
        started_at: value["started_at"].as_i64()?,
        format: Format::from_key(value["format"].as_str().unwrap_or("mono")),
        language: value["language"].as_str().unwrap_or("auto").to_owned(),
    })
}

/// Staging folders left behind with some audio in them, oldest first.
pub fn unfinished_recordings() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(crate::paths::recordings_dir()) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        // Either track counts: a machine without a microphone still records
        // the computer audio.
        .filter(|dir| raw_duration(dir) > 0)
        .collect();
    found.sort();
    found
}

/// Recorded seconds in a staging folder, from the size of the longer raw track.
pub fn raw_duration(staging: &Path) -> i64 {
    let bytes = [mic_raw(staging), system_raw(staging)]
        .iter()
        .map(|p| std::fs::metadata(p).map_or(0, |m| m.len()))
        .max()
        .unwrap_or(0);
    (bytes / u64::from(RATE * CHANNELS * 2)) as i64
}

/// The note for a staging folder, or one made up from the files.
pub fn recovery_note(staging: &Path) -> RecordingNote {
    read_recording_note(staging).unwrap_or_else(|| RecordingNote {
        title: "Recovered recording".to_owned(),
        started_at: now() - raw_duration(staging),
        format: crate::settings::load_format(),
        language: crate::settings::load_language().to_owned(),
    })
}

/// Writes the audio and the kept tracks of a stopped recording into `out`,
/// and the manifest. Returns whether the audio and the tracks were saved.
/// `progress` hears from 0 to 1 how far it is, a few times a second.
pub fn save_recording(
    staging: &Path,
    out: &Path,
    note: &RecordingNote,
    progress: impl Fn(f64),
) -> (bool, bool, Manifest) {
    use std::sync::atomic::{AtomicU64, Ordering};
    let _ = std::fs::create_dir_all(out);
    let (mic, system) = (mic_raw(staging), system_raw(staging));
    // The audio pads the shorter track with silence; done first, so the
    // kept tracks are not read while that happens.
    for path in [&mic, &system] {
        let _ = std::fs::OpenOptions::new().create(true).append(true).open(path);
    }
    let _ = export::pad_to_same_length([mic.as_path(), system.as_path()]);
    // Both tracks are read six times over (levels, the audio, the kept
    // tracks); the audio and the tracks are written side by side.
    let track = [&mic, &system]
        .iter()
        .map(|p| std::fs::metadata(p).map_or(0, |m| m.len()))
        .max()
        .unwrap_or(0);
    let total = (track * 6).max(1) as f64;
    let done = AtomicU64::new(0);
    let (audio, tracks) = std::thread::scope(|scope| {
        let audio = scope.spawn(|| export::export_audio_counted(&mic, &system, out, note.format, &done));
        let tracks = scope.spawn(|| export::export_tracks_counted(&mic, &system, out, &done));
        while !(audio.is_finished() && tracks.is_finished()) {
            progress((done.load(Ordering::Relaxed) as f64 / total).min(1.0));
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        (audio.join().unwrap_or(false), tracks.join().unwrap_or(false))
    });
    progress(1.0);
    let manifest = Manifest {
        title: note.title.clone(),
        started_at: note.started_at,
        duration_secs: raw_duration(staging),
        format: note.format,
        language: note.language.clone(),
        speakers: vec![
            crate::settings::load_your_name(),
            meeting::DEFAULT_REMOTE.to_owned(),
        ],
        labels: Vec::new(),
        imported: None,
        speaker_count: None,
        model: None,
        chapters: Vec::new(),
        chapters_by: None,
        kind: None,
        target_app: None,
    };
    let _ = meeting::write(out, &manifest);
    (audio, tracks, manifest)
}

// ---------------------------------------------------------------------------
// Import

/// The manifest of an import before it is transcribed.
pub fn import_manifest(path: &Path, language: &str, speakers: Option<usize>) -> (PathBuf, Manifest) {
    let title = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Imported audio".to_owned());
    // The file's own date says when the meeting was, better than now.
    let started_at = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(now, |d| d.as_secs() as i64);
    let out = free_output_dir(started_at, &title);
    let manifest = Manifest {
        title,
        started_at,
        duration_secs: 0,
        format: Format::Mono,
        language: language.to_owned(),
        speakers: Vec::new(),
        labels: Vec::new(),
        imported: path.file_name().map(|n| n.to_string_lossy().into_owned()),
        speaker_count: speakers,
        model: None,
        chapters: Vec::new(),
        chapters_by: None,
        kind: None,
        target_app: None,
    };
    (out, manifest)
}

/// Makes a meeting folder of an audio file: the levelled audio, and the
/// source kept for transcribing again. Returns its length in seconds.
pub fn import_audio(source: &Path, out: &Path) -> Result<i64, String> {
    std::fs::create_dir_all(out.join(export::TRACKS_DIR)).map_err(|e| e.to_string())?;
    let staging = crate::paths::cache_dir().join(format!("import-{}", now()));
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let (raw, silence) = (mic_raw(&staging), system_raw(&staging));
    let result = (|| {
        let frames = crate::codec::decode_to_raw(source, &raw)?;
        if frames == 0 {
            return Err("this file has no audio Oorapp can read".to_owned());
        }
        std::fs::write(&silence, []).map_err(|e| e.to_string())?;
        if !export::export_audio(&raw, &silence, out, Format::Mono) {
            return Err("could not convert the audio".to_owned());
        }
        if !export::export_mono(&raw, &source_track(out), 48_000) {
            return Err("could not keep the source audio".to_owned());
        }
        Ok((frames / u64::from(RATE)) as i64)
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

// ---------------------------------------------------------------------------
// Transcription

/// A line as the transcription sent it ("You: hello"), with this meeting's
/// names instead of the default labels, for the live view.
pub fn display_segment(manifest: &Manifest, text: &str) -> String {
    let Some((label, rest)) = text.split_once(": ") else {
        return text.to_owned();
    };
    match manifest.default_labels().iter().position(|l| l == label) {
        Some(i) if i < manifest.speakers.len() => format!("{}: {rest}", manifest.speakers[i]),
        _ => text.to_owned(),
    }
}

/// Transcribes `tracks` and writes `out/transcript.md`, naming the speakers
/// after `manifest` and updating it. Blocking; progress goes to `events`.
pub fn transcribe_into(
    out: &Path,
    tracks: &Tracks,
    language: &str,
    manifest: &mut Manifest,
    events: &Events,
    abort: &Abort,
) -> Result<(), String> {
    let transcript = match tracks {
        Tracks::Single(path, speakers) => transcribe::load_track(path).and_then(|track| {
            transcribe::transcribe_single(&track, language, *speakers, events, abort)
        })?,
        Tracks::Raw(dir) | Tracks::Kept(dir) => {
            let (mic_path, computer_path) = if mic_raw(dir).exists() {
                (mic_raw(dir), system_raw(dir))
            } else {
                export::tracks(dir)
            };
            let mic = transcribe::load_track(&mic_path)?;
            let computer = transcribe::load_track(&computer_path)?;
            transcribe::transcribe(&mic, &computer, language, events, abort)?
        }
    };
    let date = crate::local_date(manifest.started_at);
    let mut markdown = transcribe::to_markdown(&manifest.title, &date, &transcript);
    name_speakers(manifest, &markdown);
    // The transcription labels speakers You/Remote or Speaker N; use the
    // names of this meeting.
    let renames: Vec<(String, String)> = manifest
        .default_labels()
        .into_iter()
        .zip(manifest.speakers.iter().cloned())
        .filter(|(label, name)| label != name)
        .collect();
    markdown = meeting::relabel_all(&markdown, &renames);
    manifest.language = language.to_owned();
    manifest.model = Some(crate::models::configured());
    // Chapters of a previous transcript would point at lines that are gone.
    manifest.chapters.clear();
    manifest.chapters_by = None;
    let _ = meeting::write(out, manifest);
    std::fs::write(out.join("transcript.md"), markdown)
        .map_err(|e| format!("could not write the transcript: {e}"))
}

/// Fits the manifest's speakers to the labels in a fresh transcript.
fn name_speakers(manifest: &mut Manifest, markdown: &str) {
    if manifest.imported.is_some() {
        // An import finds its own number of speakers; keep names already
        // given and number the rest.
        let found = speakers_in(markdown);
        let mut names = manifest.speakers.clone();
        names.resize_with(found.len(), String::new);
        for (i, name) in names.iter_mut().enumerate() {
            if name.is_empty() {
                *name = format!("Speaker {}", i + 1);
            }
        }
        manifest.speakers = names;
        return;
    }
    // A recording: You, or You 1, You 2, ... when several people share the
    // mic, then Remote or Remote 1, Remote 2, ... Keep the names already given
    // to a label; a new one gets a default.
    let mut labels: Vec<String> = speakers_in(markdown)
        .into_iter()
        .filter(|l| meeting::side_of(l).is_some())
        .collect();
    labels.sort_by_key(|l| {
        let (side, n) = meeting::side_of(l).unwrap_or(("", 0));
        (side != meeting::DEFAULT_YOU, n)
    });
    if labels.is_empty() {
        return;
    }
    let known: std::collections::HashMap<String, String> = manifest
        .default_labels()
        .into_iter()
        .zip(manifest.speakers.iter().cloned())
        .collect();
    let names = labels
        .iter()
        .map(|label| {
            if let Some(name) = known.get(label) {
                return name.clone();
            }
            // One voice where there were several: the name of the first.
            if let Some(name) = known.get(&format!("{label} 1")) {
                return name.clone();
            }
            match meeting::side_of(label) {
                // The first voice on the mic is you.
                Some((side, 1)) => known.get(side).cloned().unwrap_or_else(|| label.clone()),
                Some((side, n)) if side == meeting::DEFAULT_YOU => format!("Room {n}"),
                _ => label.clone(),
            }
        })
        .collect();
    manifest.labels = labels;
    manifest.speakers = names;
}

// ---------------------------------------------------------------------------
// The transcript

/// `**[01:23] You:** text` split into its time, speaker and text.
pub fn parse_segment(line: &str) -> Option<(&str, &str, &str)> {
    let rest = line.strip_prefix("**[")?;
    let (time, rest) = rest.split_once("] ")?;
    let (speaker, text) = rest.split_once(":** ")?;
    Some((time, speaker, text.trim()))
}

/// `01:23` or `1:02:03` to milliseconds.
pub fn clock_to_ms(clock: &str) -> i64 {
    clock
        .split(':')
        .filter_map(|part| part.parse::<i64>().ok())
        .fold(0, |total, part| total * 60 + part)
        * 1000
}

/// The speaker labels in transcript Markdown, in order of first appearance.
pub fn speakers_in(markdown: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in markdown.lines() {
        if let Some((_, speaker, _)) = parse_segment(line)
            && !found.iter().any(|f| f == speaker)
        {
            found.push(speaker.to_owned());
        }
    }
    found
}

/// One paragraph of the done page: lines of one speaker close together.
#[derive(Clone, Debug)]
pub struct Paragraph {
    /// The transcript.md line numbers it was made of, for editing.
    pub sources: Vec<usize>,
    pub time: String,
    pub start_ms: i64,
    pub speaker: String,
    pub text: String,
    last_start_ms: i64,
    last_chars: usize,
}

/// The transcript as the done page shows it.
#[derive(Clone, Debug, Default)]
pub struct View {
    /// Duration and language, from the heading.
    pub meta: Vec<String>,
    /// Lines in `_italics_`: a note such as "Nothing was said".
    pub notes: Vec<String>,
    pub paragraphs: Vec<Paragraph>,
}

impl View {
    pub fn parse(markdown: &str) -> View {
        const MAX_MS: i64 = 90_000;
        let mut view = View::default();
        for (index, line) in markdown.lines().enumerate() {
            if let Some(value) = line.strip_prefix("- **Duration:** ") {
                view.meta.push(value.trim().to_owned());
            } else if let Some(value) = line.strip_prefix("- **Language:** ") {
                // Nothing was said, so nothing was detected.
                if !value.trim().eq_ignore_ascii_case("unknown") {
                    view.meta.push(value.trim().to_owned());
                }
            } else if let Some((time, speaker, text)) = parse_segment(line) {
                let start_ms = clock_to_ms(time);
                if let Some(last) = view.paragraphs.last_mut() {
                    // About 15 characters a second, plus a pause of up to 3 seconds.
                    let ended = last.last_start_ms + last.last_chars as i64 * 1000 / 15;
                    if last.speaker == speaker
                        && start_ms - ended.max(last.last_start_ms + 3000) <= 3000
                        && start_ms - last.start_ms < MAX_MS
                    {
                        last.text.push(' ');
                        last.text.push_str(text);
                        last.last_start_ms = start_ms;
                        last.last_chars = text.chars().count();
                        last.sources.push(index);
                        continue;
                    }
                }
                view.paragraphs.push(Paragraph {
                    sources: vec![index],
                    time: time.to_owned(),
                    start_ms,
                    speaker: speaker.to_owned(),
                    text: text.to_owned(),
                    last_start_ms: start_ms,
                    last_chars: text.chars().count(),
                });
            } else if let Some(note) = line.strip_prefix('_').and_then(|l| l.strip_suffix('_')) {
                view.notes.push(note.to_owned());
            }
        }
        view
    }
}

/// Changes the lines of `dir/transcript.md` and tidies the blank lines that
/// removed lines leave. Returns the transcript as it was, for an undo.
pub fn rewrite_transcript(dir: &Path, change: impl FnOnce(&mut Vec<String>)) -> Result<String, String> {
    let path = dir.join("transcript.md");
    let before = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = before.lines().map(str::to_owned).collect();
    change(&mut lines);
    let mut tidy: Vec<String> = Vec::new();
    for line in lines {
        if line.trim().is_empty() && tidy.last().is_some_and(|l| l.trim().is_empty()) {
            continue;
        }
        tidy.push(line);
    }
    std::fs::write(&path, tidy.join("\n") + "\n").map_err(|e| e.to_string())?;
    Ok(before)
}

/// Replaces a paragraph made of `sources` with one line.
pub fn edit_paragraph(dir: &Path, sources: &[usize], time: &str, who: &str, text: &str) -> Result<String, String> {
    let Some((&first, rest)) = sources.split_first() else {
        return Err("nothing to edit".into());
    };
    let line = format!("**[{time}] {who}:** {}", text.replace('\n', " ").trim());
    let rest = rest.to_vec();
    rewrite_transcript(dir, move |lines| {
        if let Some(slot) = lines.get_mut(first) {
            *slot = line;
        }
        for index in rest.into_iter().rev() {
            if index < lines.len() {
                lines.remove(index);
            }
        }
    })
}

pub fn delete_paragraph(dir: &Path, sources: &[usize]) -> Result<String, String> {
    let mut sources = sources.to_vec();
    sources.sort_unstable();
    rewrite_transcript(dir, move |lines| {
        for index in sources.into_iter().rev() {
            if index < lines.len() {
                lines.remove(index);
            }
        }
    })
}

/// Gives a paragraph to the next speaker of the meeting, for when whisper
/// guessed wrong.
pub fn swap_speaker(dir: &Path, manifest: &Manifest, sources: &[usize], who: &str) -> Result<String, String> {
    let speakers = manifest.people();
    if speakers.len() < 2 {
        return Err("only one speaker".into());
    }
    let next = speakers
        .iter()
        .position(|s| s == who)
        .map_or(0, |i| (i + 1) % speakers.len());
    let other = speakers[next].clone();
    let sources = sources.to_vec();
    rewrite_transcript(dir, move |lines| {
        for index in sources {
            if let Some(line) = lines.get_mut(index)
                && let Some((time, _, text)) = parse_segment(line)
            {
                *line = format!("**[{time}] {other}:** {text}");
            }
        }
    })
}

/// Renames the people of a meeting. `typed` has one name per entry of
/// `manifest.people()`; an empty one goes back to the default label. Two
/// people with the same name become one. Returns whether people were merged.
pub fn rename_speakers(dir: &Path, manifest: &mut Manifest, typed: &[String]) -> Result<bool, String> {
    let labels = manifest.default_labels();
    let people = manifest.people();
    let typed: Vec<String> = typed
        .iter()
        .map(|t| t.trim().replace(['*', '[', ']', ':', '\n'], ""))
        .collect();
    if typed.len() != people.len() {
        return Err("the speakers changed meanwhile".into());
    }
    let names: Vec<String> = manifest
        .speakers
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let row = people.iter().position(|p| p == name).unwrap_or(0);
            if typed[row].is_empty() {
                labels
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("Speaker {}", i + 1))
            } else {
                typed[row].clone()
            }
        })
        .collect();
    if names.is_empty() || names == manifest.speakers {
        return Ok(false);
    }
    let merged = names.iter().collect::<std::collections::HashSet<_>>().len() < people.len();
    let mut renames: Vec<(String, String)> = Vec::new();
    for (from, to) in manifest.speakers.iter().zip(&names) {
        if from != to && !renames.iter().any(|(f, _)| f == from) {
            renames.push((from.clone(), to.clone()));
        }
    }
    let transcript = dir.join("transcript.md");
    if let Ok(text) = std::fs::read_to_string(&transcript) {
        std::fs::write(&transcript, meeting::relabel_all(&text, &renames)).map_err(|e| e.to_string())?;
    }
    let you_changed = manifest.imported.is_none() && names.first() != manifest.speakers.first();
    manifest.speakers = names;
    meeting::write(dir, manifest).map_err(|e| e.to_string())?;
    if you_changed && let Some(you) = manifest.speakers.first() {
        crate::settings::save_your_name(you);
    }
    Ok(merged)
}

/// Gives the meeting a new name: the folder, the manifest and the heading in
/// the transcript follow. Returns the folder it is in now.
pub fn retitle(dir: &Path, manifest: &mut Manifest, title: &str) -> Result<PathBuf, String> {
    let title = title.trim();
    if title.is_empty() {
        return Ok(dir.to_owned());
    }
    let target = output_dir(manifest.started_at, title);
    let renamed = !folder_is_for(dir, &target);
    if renamed {
        if target.exists() {
            return Err("A meeting folder with that name already exists".into());
        }
        std::fs::rename(dir, &target).map_err(|e| format!("Could not rename the folder: {e}"))?;
    }
    let now_in = if renamed { target } else { dir.to_owned() };
    if manifest.title != title || renamed {
        manifest.title = title.to_owned();
        meeting::write(&now_in, manifest).map_err(|e| e.to_string())?;
        let transcript = now_in.join("transcript.md");
        if let Ok(text) = std::fs::read_to_string(&transcript)
            && let Some(rest) = text.strip_prefix("# ")
        {
            let body = rest.split_once('\n').map(|(_, body)| body).unwrap_or("");
            let _ = std::fs::write(&transcript, format!("# {title}\n{body}"));
        }
    }
    Ok(now_in)
}

// ---------------------------------------------------------------------------
// Chapters

/// Whether a meeting this long can get chapters, given an agent.
pub fn long_enough_for_chapters(view: &View) -> bool {
    view.paragraphs
        .last()
        .is_some_and(|p| p.start_ms >= chapters::MIN_DURATION_MS)
}

/// Asks the agent for chapters and stores them in the manifest and the
/// transcript. Blocking: the agent may take minutes.
pub fn make_chapters(dir: &Path, manifest: &mut Manifest, agent: &crate::agent::Agent) -> Result<Vec<Chapter>, String> {
    let markdown = std::fs::read_to_string(dir.join("transcript.md")).map_err(|e| e.to_string())?;
    let view = View::parse(&markdown);
    let lines: Vec<chapters::Line> = view
        .paragraphs
        .iter()
        .map(|p| chapters::Line {
            start_ms: p.start_ms,
            speaker: &p.speaker,
            text: &p.text,
        })
        .collect();
    let list = chapters::generate(agent, &lines)?;
    manifest.chapters = list.clone();
    manifest.chapters_by = Some(agent.id.clone());
    meeting::write(dir, manifest).map_err(|e| e.to_string())?;
    let transcript = dir.join("transcript.md");
    if let Ok(text) = std::fs::read_to_string(&transcript) {
        let _ = std::fs::write(&transcript, chapters::apply_to_markdown(&text, &list));
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_numbered_folder_still_belongs_to_its_name() {
        let base = Path::new("/m");
        assert!(folder_is_for(&base.join("202609241400 Weekly 2"), &base.join("202609241400 Weekly")));
        assert!(folder_is_for(&base.join("202609241400 Weekly"), &base.join("202609241400 Weekly")));
        assert!(!folder_is_for(&base.join("202609241400 Weekly sync"), &base.join("202609241400 Weekly")));
    }

    #[test]
    fn meetings_are_listed_newest_first_and_strangers_skipped() {
        let root = std::env::temp_dir().join(format!("oorapp-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // A folder from before manifests, one with a manifest, and one that is no meeting.
        std::fs::create_dir_all(root.join("202601011000 Old sync")).unwrap();
        let newer = root.join("202610111113 Weekly");
        std::fs::create_dir_all(&newer).unwrap();
        let manifest = Manifest {
            title: "Weekly".into(),
            started_at: 1_791_710_037,
            duration_secs: 60,
            format: Format::Mono,
            language: "auto".into(),
            speakers: vec!["You".into(), "Remote".into()],
            labels: Vec::new(),
            imported: None,
            speaker_count: None,
            model: None,
            chapters: Vec::new(),
            chapters_by: None,
            kind: None,
            target_app: None,
        };
        meeting::write(&newer, &manifest).unwrap();
        std::fs::create_dir_all(root.join("Holiday photos")).unwrap();
        let list = list_meetings_in(&root);
        std::fs::remove_dir_all(&root).unwrap();
        let titles: Vec<&str> = list.iter().map(|(_, m)| m.title.as_str()).collect();
        assert_eq!(titles, ["Weekly", "Old sync"]);
    }

    #[test]
    fn lines_of_one_speaker_close_together_are_one_paragraph() {
        let md = "# T\n\n**[00:00] You:** Hello.\n\n**[00:02] You:** How are you?\n\n**[00:05] Remote:** Fine.\n";
        let view = View::parse(md);
        assert_eq!(view.paragraphs.len(), 2);
        assert_eq!(view.paragraphs[0].text, "Hello. How are you?");
        assert_eq!(view.paragraphs[0].sources, vec![2, 4]);
        assert_eq!(view.paragraphs[1].start_ms, 5000);
    }
}
