//! The Oorapp window on top of the core: one recorder, the meeting it shows,
//! the menu-bar item while it records, and commands for the Vue front end.
//!
//! State lives in `Hub`. Long work (saving, transcribing, chapters, actions,
//! the model download) runs on threads and reports through events:
//!
//! | event | payload |
//! |---|---|
//! | `status` | `Status`, whenever the phase or the progress changes |
//! | `levels` | `Levels`, 25 times a second |
//! | `meeting` | nothing; the meeting on disk changed, fetch it again |
//! | `toast` | a short message |

mod notes;
mod tray;
mod updates;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use oorapp_capture::{Kind, Source};
use oorapp_core::export::Format;
use oorapp_core::meeting::{self, Manifest};
use oorapp_core::session::{self, RecordingNote, Tracks, View};
use oorapp_core::transcribe::{Abort, CANCELLED, Event, LANGUAGES};
use oorapp_core::{actions, agent, models, settings};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

/// Shown at least this long, also for a short recording, so it reads as a
/// step rather than a flicker.
const MIN_ANIMATION: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Idle,
    Recording,
    Saving,
    Transcribing,
    Done,
}

pub struct Hub {
    mic: Source,
    computer: Source,
    inner: Mutex<Inner>,
}

struct Inner {
    phase: Phase,
    paused: bool,
    title: String,
    format: Format,
    language: String,
    started_at: i64,
    paused_secs: i64,
    pause_began: i64,
    staging: Option<PathBuf>,
    dir: Option<PathBuf>,
    manifest: Option<Manifest>,
    stage: String,
    progress: f64,
    lines: VecDeque<String>,
    problem: Option<String>,
    abort: Option<Abort>,
    chapters_busy: bool,
    chapters_note: Option<String>,
    model_download: Option<f64>,
    action_running: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct Status {
    phase: Phase,
    paused: bool,
    title: String,
    format: String,
    language: String,
    elapsed: i64,
    dir: Option<String>,
    stage: String,
    progress: f64,
    lines: Vec<String>,
    problem: Option<String>,
    /// The speech model's name and size when it still has to be downloaded.
    model_missing: Option<(String, u32)>,
    model_download: Option<f64>,
    mic_device: Option<String>,
    computer_device: Option<String>,
    mic_problem: Option<String>,
    computer_problem: Option<String>,
    /// Seconds the computer audio has been exact silence: on macOS a hint
    /// that the System Audio Recording permission is missing.
    computer_silent: u64,
    chapters_busy: bool,
    action_running: Option<String>,
    /// Seconds into a quick note being spoken.
    note: Option<i64>,
    /// A note is being turned into text.
    note_busy: bool,
    note_permissions: notes::Permissions,
}

#[derive(Clone, Serialize)]
struct Levels {
    mic: Vec<f32>,
    computer: Vec<f32>,
    elapsed: i64,
}

impl Hub {
    fn new() -> Self {
        Hub {
            mic: Source::spawn(Kind::Mic),
            computer: Source::spawn(Kind::Computer),
            inner: Mutex::new(Inner {
                phase: Phase::Idle,
                paused: false,
                title: String::new(),
                format: settings::load_format(),
                language: settings::load_language().to_owned(),
                started_at: 0,
                paused_secs: 0,
                pause_began: 0,
                staging: None,
                dir: None,
                manifest: None,
                stage: String::new(),
                progress: 0.0,
                lines: VecDeque::new(),
                problem: None,
                abort: None,
                chapters_busy: false,
                chapters_note: None,
                model_download: None,
                action_running: None,
            }),
        }
    }

    fn status(&self) -> Status {
        let inner = self.inner.lock().unwrap();
        Status {
            phase: inner.phase,
            paused: inner.paused,
            title: inner.title.clone(),
            format: inner.format.key().to_owned(),
            language: inner.language.clone(),
            elapsed: elapsed(&inner),
            dir: inner.dir.as_ref().map(|d| d.display().to_string()),
            stage: inner.stage.clone(),
            progress: inner.progress,
            lines: inner.lines.iter().cloned().collect(),
            problem: inner.problem.clone(),
            model_missing: models::missing().map(|(name, size)| (name.to_owned(), size)),
            model_download: inner.model_download,
            mic_device: self.mic.device(),
            computer_device: self.computer.device(),
            mic_problem: self.mic.problem(),
            computer_problem: self.computer.problem(),
            computer_silent: self.computer.digital_silence().as_secs(),
            chapters_busy: inner.chapters_busy,
            action_running: inner.action_running.clone(),
            note: None,
            note_busy: false,
            note_permissions: notes::Permissions { keys: true, paste: true },
        }
    }
}

fn elapsed(inner: &Inner) -> i64 {
    if inner.phase != Phase::Recording {
        return 0;
    }
    let now = session::now();
    let paused_now = if inner.paused { now - inner.pause_began } else { 0 };
    (now - inner.started_at - inner.paused_secs - paused_now).max(0)
}

fn full_status(app: &AppHandle) -> Status {
    let mut status = app.state::<Hub>().status();
    (status.note, status.note_busy) = notes::status(app);
    status.note_permissions = notes::permissions();
    status
}

fn emit_status(app: &AppHandle) {
    let status = full_status(app);
    tray::update(app, &status);
    let _ = app.emit("status", status);
}

fn toast(app: &AppHandle, message: impl Into<String>) {
    let _ = app.emit("toast", message.into());
}

// ---------------------------------------------------------------------------
// Settings

#[derive(Serialize)]
struct Choice {
    key: String,
    label: String,
}

#[derive(Serialize)]
struct Options {
    formats: Vec<Choice>,
    languages: Vec<Choice>,
    agent: Option<String>,
    agent_problem: Option<String>,
    actions: Vec<String>,
    actions_docs: &'static str,
    config_file: String,
    note_shortcut: String,
    note_shortcuts: Vec<Choice>,
    meetings_dir: String,
    platform: &'static str,
}

#[tauri::command]
fn options() -> Options {
    let agent = agent::status();
    Options {
        formats: Format::ALL
            .iter()
            .map(|f| Choice {
                key: f.key().into(),
                label: f.label().into(),
            })
            .collect(),
        languages: LANGUAGES
            .iter()
            .map(|(code, label)| Choice {
                key: (*code).into(),
                label: (*label).into(),
            })
            .collect(),
        agent: agent.as_ref().ok().map(|a| a.name.to_owned()),
        agent_problem: agent.err().map(|e| e.to_string()),
        actions: actions::load().into_iter().map(|a| a.name).collect(),
        actions_docs: actions::DOCS,
        config_file: models::config_file().display().to_string(),
        note_shortcut: notes::shortcut(),
        note_shortcuts: notes::choices()
            .into_iter()
            .map(|(key, label)| Choice { key, label })
            .collect(),
        meetings_dir: oorapp_core::paths::meetings_dir().display().to_string(),
        platform: std::env::consts::OS,
    }
}

#[tauri::command]
fn status(app: AppHandle) -> Status {
    full_status(&app)
}

#[tauri::command]
fn set_format(app: AppHandle, hub: State<Hub>, key: String) {
    let format = Format::from_key(&key);
    settings::save_format(format);
    hub.inner.lock().unwrap().format = format;
    emit_status(&app);
}

#[tauri::command]
fn set_language(app: AppHandle, hub: State<Hub>, code: String) {
    if LANGUAGES.iter().any(|(c, _)| *c == code) {
        settings::save_language(&code);
        hub.inner.lock().unwrap().language = code;
    }
    emit_status(&app);
}

/// The name of the meeting. Before and during a recording it is only
/// remembered; on the done page the folder and the transcript follow.
#[tauri::command]
fn set_title(app: AppHandle, hub: State<Hub>, title: String) -> Result<(), String> {
    let title = title.trim().to_owned();
    let mut inner = hub.inner.lock().unwrap();
    inner.title = title.clone();
    let dir = inner.dir.clone();
    let retitle = inner.phase == Phase::Done
        && !title.is_empty()
        && inner.manifest.as_ref().is_some_and(|m| m.title != title);
    if retitle
        && let (Some(dir), Some(manifest)) = (dir, inner.manifest.as_mut())
    {
        let moved = session::retitle(&dir, manifest, &title)?;
        inner.dir = Some(moved);
        drop(inner);
        let _ = app.emit("meeting", ());
        toast(&app, "Saved");
    } else {
        drop(inner);
    }
    emit_status(&app);
    Ok(())
}

// ---------------------------------------------------------------------------
// Recording

#[tauri::command]
fn start(app: AppHandle, title: Option<String>) -> Result<(), String> {
    start_recording(&app, title)
}

pub fn start_recording(app: &AppHandle, title: Option<String>) -> Result<(), String> {
    let hub = app.state::<Hub>();
    let mut inner = hub.inner.lock().unwrap();
    if matches!(inner.phase, Phase::Recording | Phase::Saving | Phase::Transcribing) {
        return Err("Finish the current recording first".into());
    }
    if let Some(title) = title.filter(|t| !t.trim().is_empty()) {
        inner.title = title.trim().to_owned();
    }
    if inner.title.is_empty() || inner.phase == Phase::Done {
        inner.title = session::default_title();
    }
    let started_at = session::now();
    let staging = session::new_staging(started_at).map_err(|e| e.to_string())?;
    if let Err(e) = hub
        .mic
        .start_recording(&session::mic_raw(&staging))
        .and_then(|()| hub.computer.start_recording(&session::system_raw(&staging)))
    {
        hub.mic.stop_recording();
        hub.computer.stop_recording();
        return Err(format!("Could not start recording: {e}"));
    }
    session::write_recording_note(
        &staging,
        &RecordingNote {
            title: inner.title.clone(),
            started_at,
            format: inner.format,
            language: inner.language.clone(),
        },
    );
    inner.staging = Some(staging);
    inner.dir = None;
    inner.manifest = None;
    inner.started_at = started_at;
    inner.paused = false;
    inner.paused_secs = 0;
    inner.problem = None;
    inner.phase = Phase::Recording;
    drop(inner);
    emit_status(app);
    Ok(())
}

#[tauri::command]
fn toggle_pause(app: AppHandle) {
    pause_recording(&app);
}

pub fn pause_recording(app: &AppHandle) {
    let hub = app.state::<Hub>();
    let mut inner = hub.inner.lock().unwrap();
    if inner.phase != Phase::Recording {
        return;
    }
    let now = session::now();
    if inner.paused {
        inner.paused_secs += now - inner.pause_began;
        inner.paused = false;
    } else {
        inner.pause_began = now;
        inner.paused = true;
    }
    hub.mic.set_paused(inner.paused);
    hub.computer.set_paused(inner.paused);
    drop(inner);
    emit_status(app);
}

#[tauri::command]
fn stop(app: AppHandle) {
    stop_recording(&app);
}

pub fn stop_recording(app: &AppHandle) {
    let hub = app.state::<Hub>();
    let mut inner = hub.inner.lock().unwrap();
    if inner.phase != Phase::Recording {
        return;
    }
    hub.mic.stop_recording();
    hub.computer.stop_recording();
    let Some(staging) = inner.staging.clone() else {
        return;
    };
    let note = RecordingNote {
        title: if inner.title.is_empty() {
            session::default_title()
        } else {
            inner.title.clone()
        },
        started_at: inner.started_at,
        format: inner.format,
        language: inner.language.clone(),
    };
    inner.paused = false;
    drop(inner);
    finish_recording(app.clone(), staging, note);
}

/// Saves a stopped (or recovered) recording and transcribes it, on a thread.
fn finish_recording(app: AppHandle, staging: PathBuf, note: RecordingNote) {
    begin_work(&app, Phase::Saving, "Saving audio");
    std::thread::spawn(move || {
        let since = Instant::now();
        let out = session::free_output_dir(note.started_at, &note.title);
        let (audio, tracks, manifest) = session::save_recording(&staging, &out, &note);
        {
            let hub = app.state::<Hub>();
            let mut inner = hub.inner.lock().unwrap();
            inner.dir = Some(out.clone());
            inner.manifest = Some(manifest);
            inner.title = note.title.clone();
        }
        let result = run_transcription(&app, &out, Tracks::Raw(staging.clone()), &note.language);
        // The kept tracks are enough to transcribe again; the raw files can go.
        if audio && tracks {
            let _ = std::fs::remove_dir_all(&staging);
        }
        finished(&app, since, audio, result);
    });
}

fn begin_work(app: &AppHandle, phase: Phase, stage: &str) {
    let hub = app.state::<Hub>();
    let mut inner = hub.inner.lock().unwrap();
    inner.phase = phase;
    inner.stage = stage.to_owned();
    inner.progress = 0.0;
    inner.lines.clear();
    inner.problem = None;
    inner.chapters_note = None;
    drop(inner);
    emit_status(app);
}

/// Transcribes into `out`, reporting progress. Blocking.
fn run_transcription(app: &AppHandle, out: &Path, tracks: Tracks, language: &str) -> Result<(), String> {
    let hub = app.state::<Hub>();
    let abort = Abort::default();
    let mut manifest = {
        let mut inner = hub.inner.lock().unwrap();
        inner.phase = Phase::Transcribing;
        inner.stage = "Loading audio".into();
        inner.abort = Some(abort.clone());
        inner.manifest.clone()
    }
    .ok_or("no meeting")?;
    emit_status(app);

    let (tx, rx) = async_channel::unbounded::<Event>();
    let reporter = {
        let app = app.clone();
        let names = manifest.clone();
        std::thread::spawn(move || {
            let mut last_emit = Instant::now();
            while let Ok(event) = rx.recv_blocking() {
                let hub = app.state::<Hub>();
                let mut inner = hub.inner.lock().unwrap();
                match event {
                    Event::Stage(stage) => inner.stage = stage,
                    Event::Progress(p) => inner.progress = p,
                    Event::Segment(text) => {
                        inner.lines.push_back(session::display_segment(&names, &text));
                        while inner.lines.len() > 14 {
                            inner.lines.pop_front();
                        }
                    }
                    Event::Finished => break,
                }
                drop(inner);
                if last_emit.elapsed() > Duration::from_millis(80) {
                    last_emit = Instant::now();
                    emit_status(&app);
                }
            }
        })
    };
    let result = session::transcribe_into(out, &tracks, language, &mut manifest, &tx, &abort);
    let _ = tx.send_blocking(Event::Finished);
    let _ = reporter.join();
    let mut inner = hub.inner.lock().unwrap();
    inner.abort = None;
    if result.is_ok() {
        inner.manifest = Some(manifest);
    }
    result
}

/// Ends the transcribing phase: holds the animation, then shows the meeting
/// and starts on the chapters and the automatic actions.
fn finished(app: &AppHandle, since: Instant, audio_ok: bool, result: Result<(), String>) {
    let hub = app.state::<Hub>();
    let cancelled = matches!(&result, Err(m) if m == CANCELLED);
    if !cancelled && since.elapsed() < MIN_ANIMATION {
        {
            let mut inner = hub.inner.lock().unwrap();
            inner.progress = 1.0;
            inner.stage = "Done".into();
        }
        emit_status(app);
        std::thread::sleep(MIN_ANIMATION - since.elapsed());
    }
    let problem = match (&result, audio_ok) {
        (Err(_), _) if cancelled => Some("Transcription cancelled.".to_owned()),
        (Err(message), _) => Some(format!("Transcription failed: {message}.")),
        (Ok(()), false) => Some("Could not save the audio.".to_owned()),
        _ => None,
    };
    {
        let mut inner = hub.inner.lock().unwrap();
        inner.phase = Phase::Done;
        inner.problem = problem.clone();
    }
    emit_status(app);
    let _ = app.emit("meeting", ());
    if problem.is_none() {
        after_transcript(app.clone());
    }
}

/// Chapters when an agent is around and the meeting is long enough, then
/// the actions that run on their own.
fn after_transcript(app: AppHandle) {
    std::thread::spawn(move || {
        let hub = app.state::<Hub>();
        let dir = hub.inner.lock().unwrap().dir.clone();
        let long = dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join("transcript.md")).ok())
            .is_some_and(|md| session::long_enough_for_chapters(&View::parse(&md)));
        if long && agent::default_agent().is_some() {
            chapters_now(&app);
        }
        for action in actions::automatic() {
            perform(&app, action);
        }
    });
}

#[tauri::command]
fn cancel(hub: State<Hub>) {
    if let Some(abort) = hub.inner.lock().unwrap().abort.as_ref() {
        abort.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Back to the start page, ready for the next meeting.
#[tauri::command]
fn new_recording(app: AppHandle, hub: State<Hub>) {
    let mut inner = hub.inner.lock().unwrap();
    if matches!(inner.phase, Phase::Recording | Phase::Saving | Phase::Transcribing) {
        return;
    }
    inner.phase = Phase::Idle;
    inner.dir = None;
    inner.manifest = None;
    inner.title.clear();
    inner.problem = None;
    drop(inner);
    emit_status(&app);
}

// ---------------------------------------------------------------------------
// Import, recovery, opening

#[tauri::command]
fn import_file(app: AppHandle, path: String, language: String, speakers: Option<usize>) -> Result<(), String> {
    let hub = app.state::<Hub>();
    {
        let inner = hub.inner.lock().unwrap();
        if matches!(inner.phase, Phase::Recording | Phase::Saving | Phase::Transcribing) {
            return Err("Finish the current recording first".into());
        }
    }
    let source = PathBuf::from(&path);
    let (out, manifest) = session::import_manifest(&source, &language, speakers);
    {
        let mut inner = hub.inner.lock().unwrap();
        inner.title = manifest.title.clone();
        inner.dir = Some(out.clone());
        inner.manifest = Some(manifest);
    }
    begin_work(&app, Phase::Saving, "Importing audio");
    std::thread::spawn(move || {
        let since = Instant::now();
        let result = session::import_audio(&source, &out).and_then(|secs| {
            {
                let hub = app.state::<Hub>();
                let mut inner = hub.inner.lock().unwrap();
                if let Some(m) = inner.manifest.as_mut() {
                    m.duration_secs = secs;
                    let _ = meeting::write(&out, m);
                }
            }
            run_transcription(&app, &out, Tracks::Single(session::source_track(&out), speakers), &language)
        });
        finished(&app, since, result.is_ok(), result);
    });
    Ok(())
}

#[derive(Serialize)]
struct Unfinished {
    path: String,
    title: String,
    started_at: i64,
    seconds: i64,
}

#[tauri::command]
fn unfinished(hub: State<Hub>) -> Vec<Unfinished> {
    // The recording that is running now is not unfinished.
    let (current, busy) = {
        let inner = hub.inner.lock().unwrap();
        (inner.staging.clone(), inner.phase != Phase::Idle && inner.phase != Phase::Done)
    };
    session::unfinished_recordings()
        .into_iter()
        .filter(|dir| !(busy && Some(dir) == current.as_ref()))
        .map(|dir| {
            let note = session::recovery_note(&dir);
            Unfinished {
                path: dir.display().to_string(),
                title: note.title,
                started_at: note.started_at,
                seconds: session::raw_duration(&dir),
            }
        })
        .collect()
}

#[tauri::command]
fn recover(app: AppHandle, path: String) -> Result<(), String> {
    let staging = PathBuf::from(path);
    if !staging.starts_with(oorapp_core::paths::recordings_dir()) {
        return Err("not a recording".into());
    }
    {
        let hub = app.state::<Hub>();
        let inner = hub.inner.lock().unwrap();
        if inner.phase != Phase::Idle && inner.phase != Phase::Done {
            return Err("Finish the current recording first".into());
        }
    }
    let note = session::recovery_note(&staging);
    finish_recording(app, staging, note);
    Ok(())
}

#[tauri::command]
fn discard(path: String) -> Result<(), String> {
    let staging = PathBuf::from(path);
    if !staging.starts_with(oorapp_core::paths::recordings_dir()) {
        return Err("not a recording".into());
    }
    std::fs::remove_dir_all(staging).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct MeetingEntry {
    path: String,
    title: String,
    started_at: i64,
    duration_secs: i64,
    /// The meeting this window shows now.
    current: bool,
    /// "note", "file" or "meeting".
    kind: &'static str,
    text: String,
    words: usize,
    transcribed: bool,
    target_app: Option<String>,
}

/// Everything in the meetings folder, newest first, for the history.
#[tauri::command]
async fn meetings(hub: State<'_, Hub>) -> Result<Vec<MeetingEntry>, String> {
    let current = {
        let inner = hub.inner.lock().unwrap();
        if inner.phase == Phase::Done { inner.dir.clone() } else { None }
    };
    tauri::async_runtime::spawn_blocking(move || {
        session::history()
            .into_iter()
            .map(|e| MeetingEntry {
                current: current.as_ref() == Some(&e.path),
                path: e.path.display().to_string(),
                title: e.title,
                started_at: e.started_at,
                duration_secs: e.duration_secs,
                kind: e.kind,
                text: e.text,
                words: e.words,
                transcribed: e.transcribed,
                target_app: e.target_app,
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

/// A meeting's transcript as Markdown, for copying from the history; a
/// note's text without the heading.
#[tauri::command]
fn meeting_text(path: String) -> Result<String, String> {
    let (dir, manifest) = meeting::open(Path::new(&path)).ok_or("This is not a meeting")?;
    let entry = session::history_entry(&dir, &manifest);
    if entry.kind == "note" {
        return Ok(entry.text);
    }
    std::fs::read_to_string(dir.join("transcript.md")).map_err(|e| e.to_string())
}

/// Moves a meeting to the trash. Not the one being shown or worked on.
#[tauri::command]
fn trash_meeting(app: AppHandle, path: String) -> Result<(), String> {
    let dir = PathBuf::from(&path);
    {
        let hub = app.state::<Hub>();
        let mut inner = hub.inner.lock().unwrap();
        if inner.dir.as_deref() == Some(dir.as_path()) {
            if inner.phase != Phase::Done && inner.phase != Phase::Idle {
                return Err("This meeting is still being saved or transcribed".into());
            }
            // The open meeting is closed first.
            inner.phase = Phase::Idle;
            inner.dir = None;
            inner.manifest = None;
            inner.title.clear();
        }
    }
    emit_status(&app);
    session::trash_meeting(&dir)
}

/// Opens a meeting folder in Finder or Explorer. From Rust, so it needs no
/// path scope in the front end.
#[tauri::command]
fn open_folder(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = PathBuf::from(path);
    if !dir.is_dir() {
        return Err("That folder is gone".into());
    }
    app.opener().open_path(dir.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

/// Puts text on the clipboard. Done here because the web view only allows it
/// right after a click, not after waiting for the text.
#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .map_err(|e| e.to_string())
}

/// Copies a meeting's transcript, or a note's text.
#[tauri::command]
fn copy_meeting(path: String) -> Result<(), String> {
    copy_text(meeting_text(path)?)
}

#[tauri::command]
fn set_note_shortcut(app: AppHandle, shortcut: String) -> Result<(), String> {
    oorapp_core::settings::save_note_shortcut(&shortcut);
    notes::apply_shortcut(&app)
}

#[tauri::command]
fn request_permission(app: AppHandle, which: String) {
    notes::request_permission(&app, &which);
}

/// Connects Oorapp's MCP server to Claude: "code" registers it with the
/// Claude Code CLI for every project, "desktop" adds it to Claude Desktop's
/// config file. Both point at this very executable with the `mcp` argument.
#[tauri::command]
fn connect_claude(target: String) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    match target.as_str() {
        "code" => {
            let output = std::process::Command::new("claude")
                .env("PATH", oorapp_core::paths::search_path())
                .args(["mcp", "add", "--scope", "user", "oorapp", "--"])
                .arg(&exe)
                .arg("mcp")
                .output()
                .map_err(|_| "Claude Code is not installed (no `claude` command found)".to_string())?;
            let text = String::from_utf8_lossy(&output.stdout).to_string()
                + &String::from_utf8_lossy(&output.stderr);
            if output.status.success() || text.contains("already exists") {
                Ok("Oorapp is connected to Claude Code".into())
            } else {
                Err(text.lines().last().unwrap_or("Claude Code refused").to_owned())
            }
        }
        "desktop" => {
            let config = claude_desktop_config().ok_or("Claude Desktop is not installed")?;
            let mut value: serde_json::Value = std::fs::read_to_string(&config)
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_else(|| serde_json::json!({}));
            if !value.is_object() {
                return Err("Claude Desktop's config file is not valid JSON".into());
            }
            value["mcpServers"]["oorapp"] = serde_json::json!({
                "command": exe.display().to_string(),
                "args": ["mcp"],
            });
            if let Some(dir) = config.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            std::fs::write(&config, serde_json::to_string_pretty(&value).unwrap_or_default())
                .map_err(|e| e.to_string())?;
            Ok("Added to Claude Desktop; restart Claude to use it".into())
        }
        _ => Err("unknown target".into()),
    }
}

fn claude_desktop_config() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let dir = dirs_home().join("Library/Application Support/Claude");
    #[cfg(windows)]
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("Claude");
    #[cfg(not(any(target_os = "macos", windows)))]
    let dir = dirs_home().join(".config/Claude");
    dir.is_dir().then(|| dir.join("claude_desktop_config.json"))
}

fn dirs_home() -> PathBuf {
    oorapp_core::paths::home()
}

/// Restarts Oorapp, for a permission macOS only shows a new process.
#[tauri::command]
fn restart_app(app: AppHandle) -> Result<(), String> {
    let busy = matches!(
        app.state::<Hub>().inner.lock().unwrap().phase,
        Phase::Recording | Phase::Saving | Phase::Transcribing
    );
    if busy {
        return Err("Finish the recording first".into());
    }
    app.restart();
}

#[tauri::command]
fn toggle_note(app: AppHandle) {
    notes::toggle(&app);
}

#[tauri::command]
fn open_meeting(app: AppHandle, path: String) -> Result<(), String> {
    open_path(&app, Path::new(&path))
}

pub fn open_path(app: &AppHandle, path: &Path) -> Result<(), String> {
    let (dir, manifest) = meeting::open(path).ok_or("This is not a meeting")?;
    let hub = app.state::<Hub>();
    let mut inner = hub.inner.lock().unwrap();
    if matches!(inner.phase, Phase::Recording | Phase::Saving | Phase::Transcribing) {
        return Err("Finish the current recording first".into());
    }
    if meeting::find(&dir).is_none() {
        let _ = meeting::write(&dir, &manifest);
    }
    inner.title = manifest.title.clone();
    inner.dir = Some(dir);
    inner.manifest = Some(manifest);
    inner.phase = Phase::Done;
    inner.problem = None;
    drop(inner);
    emit_status(app);
    let _ = app.emit("meeting", ());
    Ok(())
}

// ---------------------------------------------------------------------------
// The meeting on the done page

#[derive(Serialize)]
struct SpeakerRow {
    /// "Speaker on the microphone", "Speaker 2", ...
    role: String,
    name: String,
}

#[derive(Serialize)]
struct ParagraphRow {
    sources: Vec<usize>,
    time: String,
    start_ms: i64,
    speaker: String,
    /// Index into `speakers` for the colour.
    colour: usize,
    text: String,
}

#[derive(Serialize)]
struct ChapterRow {
    start_ms: i64,
    clock: String,
    title: String,
}

#[derive(Serialize)]
struct MeetingData {
    dir: String,
    title: String,
    started_at: i64,
    duration_secs: i64,
    meta: Vec<String>,
    notes: Vec<String>,
    language: String,
    imported: bool,
    speakers: Vec<SpeakerRow>,
    paragraphs: Vec<ParagraphRow>,
    chapters: Vec<ChapterRow>,
    chapters_note: Option<String>,
    can_have_chapters: bool,
    markdown: String,
    /// Audio to play: one file, or the mic and the computer audio as two.
    audio: Vec<String>,
}

#[tauri::command]
fn meeting_data(hub: State<Hub>) -> Option<MeetingData> {
    let inner = hub.inner.lock().unwrap();
    let dir = inner.dir.clone()?;
    let manifest = inner.manifest.clone()?;
    let chapters_note = inner.chapters_note.clone();
    drop(inner);
    let markdown = std::fs::read_to_string(dir.join("transcript.md")).unwrap_or_default();
    let view = View::parse(&markdown);
    let labels = manifest.default_labels();
    let people = manifest.people();
    let speakers = people
        .iter()
        .map(|name| {
            let i = manifest.speakers.iter().position(|s| s == name).unwrap_or(0);
            let side = labels.get(i).and_then(|l| meeting::side_of(l));
            let role = match (manifest.imported.is_some(), side) {
                (true, _) | (false, None) => format!("Speaker {}", i + 1),
                (false, Some((side, n))) => {
                    let place = if side == meeting::DEFAULT_YOU {
                        "the microphone"
                    } else {
                        "the computer audio"
                    };
                    if n == 0 {
                        format!("Speaker on {place}")
                    } else {
                        format!("Speaker {n} on {place}")
                    }
                }
            };
            SpeakerRow {
                role,
                name: name.clone(),
            }
        })
        .collect();
    // Colours follow the speaker order of the meeting, then first appearance.
    let mut order = people.clone();
    for p in &view.paragraphs {
        if !order.contains(&p.speaker) {
            order.push(p.speaker.clone());
        }
    }
    let paragraphs = view
        .paragraphs
        .iter()
        .map(|p| ParagraphRow {
            sources: p.sources.clone(),
            time: p.time.clone(),
            start_ms: p.start_ms,
            speaker: p.speaker.clone(),
            colour: order.iter().position(|s| *s == p.speaker).unwrap_or(0),
            text: p.text.clone(),
        })
        .collect();
    let mut meta = view.meta.clone();
    meta.push(manifest.format.short_label().to_owned());
    let audio = ["audio.ogg", "mic.ogg", "computer.ogg"]
        .iter()
        .map(|f| dir.join(f))
        .filter(|p| p.exists())
        .map(|p| p.display().to_string())
        .collect();
    Some(MeetingData {
        dir: dir.display().to_string(),
        title: manifest.title.clone(),
        started_at: manifest.started_at,
        duration_secs: manifest.duration_secs,
        meta,
        notes: view.notes.clone(),
        language: manifest.language.clone(),
        imported: manifest.imported.is_some(),
        speakers,
        paragraphs,
        chapters: manifest
            .chapters
            .iter()
            .map(|c| ChapterRow {
                start_ms: c.start_ms,
                clock: oorapp_core::chapters::clock(c.start_ms),
                title: c.title.clone(),
            })
            .collect(),
        chapters_note,
        can_have_chapters: session::long_enough_for_chapters(&view),
        markdown,
        audio,
    })
}

/// Loudness over time of the mic and the computer audio, for the player:
/// `bins` peaks each, 0..1. An import has one track, returned as `mic`.
#[derive(Serialize)]
struct Peaks {
    mic: Vec<f32>,
    computer: Vec<f32>,
}

#[tauri::command]
async fn peaks(hub: State<'_, Hub>, bins: usize) -> Result<Peaks, String> {
    let (dir, imported) = {
        let inner = hub.inner.lock().unwrap();
        (
            inner.dir.clone().ok_or("no meeting")?,
            inner.manifest.as_ref().is_some_and(|m| m.imported.is_some()),
        )
    };
    let bins = bins.clamp(50, 4000);
    tauri::async_runtime::spawn_blocking(move || {
        let (mic, computer) = oorapp_core::export::tracks(&dir);
        if imported {
            let source = session::source_track(&dir);
            return Peaks {
                mic: peaks_of(&source, bins),
                computer: Vec::new(),
            };
        }
        if mic.exists() {
            return Peaks {
                mic: peaks_of(&mic, bins),
                computer: peaks_of(&computer, bins),
            };
        }
        // An older meeting without kept tracks: the audio as one.
        let audio = ["audio.ogg", "mic.ogg"].iter().map(|f| dir.join(f)).find(|p| p.exists());
        Peaks {
            mic: audio.map(|a| peaks_of(&a, bins)).unwrap_or_default(),
            computer: Vec::new(),
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// The loudest sample per bin, scaled to 0..1 with a gentle curve so quiet
/// speech still shows.
fn peaks_of(path: &Path, bins: usize) -> Vec<f32> {
    let Ok(samples) = oorapp_core::codec::decode_mono(path, 4000) else {
        return vec![0.0; bins];
    };
    if samples.is_empty() {
        return vec![0.0; bins];
    }
    let per_bin = samples.len().div_ceil(bins);
    let mut out: Vec<f32> = samples
        .chunks(per_bin)
        .map(|c| c.iter().fold(0.0f32, |m, v| m.max(v.abs())))
        .collect();
    out.resize(bins, 0.0);
    let loudest = out.iter().copied().fold(0.0, f32::max).max(1e-4);
    out.into_iter().map(|v| (v / loudest).sqrt()).collect()
}

fn with_meeting<T>(
    app: &AppHandle,
    change: impl FnOnce(&Path, &mut Manifest) -> Result<T, String>,
) -> Result<T, String> {
    let hub = app.state::<Hub>();
    let mut inner = hub.inner.lock().unwrap();
    let dir = inner.dir.clone().ok_or("no meeting")?;
    let manifest = inner.manifest.as_mut().ok_or("no meeting")?;
    let result = change(&dir, manifest);
    drop(inner);
    let _ = app.emit("meeting", ());
    result
}

#[tauri::command]
fn edit_paragraph(app: AppHandle, sources: Vec<usize>, time: String, speaker: String, text: String) -> Result<String, String> {
    with_meeting(&app, |dir, _| session::edit_paragraph(dir, &sources, &time, &speaker, &text))
}

#[tauri::command]
fn delete_paragraph(app: AppHandle, sources: Vec<usize>) -> Result<String, String> {
    with_meeting(&app, |dir, _| session::delete_paragraph(dir, &sources))
}

#[tauri::command]
fn swap_speaker(app: AppHandle, sources: Vec<usize>, speaker: String) -> Result<String, String> {
    with_meeting(&app, |dir, m| session::swap_speaker(dir, m, &sources, &speaker))
}

/// Puts back the transcript as it was before an edit.
#[tauri::command]
fn restore_transcript(app: AppHandle, markdown: String) -> Result<(), String> {
    with_meeting(&app, |dir, _| {
        std::fs::write(dir.join("transcript.md"), markdown).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn rename_speakers(app: AppHandle, names: Vec<String>) -> Result<bool, String> {
    with_meeting(&app, |dir, m| session::rename_speakers(dir, m, &names))
}

#[tauri::command]
fn transcribe_again(app: AppHandle, language: String) -> Result<(), String> {
    let hub = app.state::<Hub>();
    let (dir, tracks) = {
        let mut inner = hub.inner.lock().unwrap();
        if inner.phase != Phase::Done {
            return Err("Nothing to transcribe".into());
        }
        let dir = inner.dir.clone().ok_or("no meeting")?;
        let manifest = inner.manifest.as_ref().ok_or("no meeting")?;
        let tracks = session::kept_tracks(&dir, manifest);
        inner.language = language.clone();
        (dir, tracks)
    };
    begin_work(&app, Phase::Transcribing, "Loading audio");
    std::thread::spawn(move || {
        let since = Instant::now();
        let result = run_transcription(&app, &dir, tracks, &language);
        finished(&app, since, true, result);
    });
    Ok(())
}

#[tauri::command]
fn make_chapters(app: AppHandle) {
    std::thread::spawn(move || chapters_now(&app));
}

fn chapters_now(app: &AppHandle) {
    let Some(agent) = agent::default_agent() else {
        return;
    };
    let hub = app.state::<Hub>();
    let (dir, mut manifest) = {
        let mut inner = hub.inner.lock().unwrap();
        if inner.chapters_busy {
            return;
        }
        let (Some(dir), Some(manifest)) = (inner.dir.clone(), inner.manifest.clone()) else {
            return;
        };
        inner.chapters_busy = true;
        inner.chapters_note = Some(format!("Writing chapters with {}…", agent.name));
        (dir, manifest)
    };
    emit_status(app);
    let _ = app.emit("meeting", ());
    let result = session::make_chapters(&dir, &mut manifest, &agent);
    {
        let mut inner = hub.inner.lock().unwrap();
        inner.chapters_busy = false;
        // The meeting may have been renamed (same timestamp) or replaced meanwhile.
        let same = inner
            .dir
            .as_ref()
            .and_then(|d| d.file_name())
            .zip(dir.file_name())
            .is_some_and(|(a, b)| a.to_string_lossy().get(..12) == b.to_string_lossy().get(..12));
        match &result {
            Ok(list) if same => {
                if let Some(m) = inner.manifest.as_mut() {
                    m.chapters = list.clone();
                    m.chapters_by = Some(agent.id.clone());
                }
                inner.chapters_note = Some(format!("Made with {}", agent.name));
            }
            Ok(_) => {}
            Err(message) => {
                eprintln!("oorapp: chapters: {message}");
                inner.chapters_note = Some(format!("{} could not make chapters", agent.name));
            }
        }
    }
    emit_status(app);
    let _ = app.emit("meeting", ());
    if let Ok(list) = result {
        toast(app, format!("{} chapters added", list.len()));
    }
}

#[derive(Clone, Serialize)]
struct ActionResult {
    message: String,
    url: Option<String>,
}

#[tauri::command]
fn run_action(app: AppHandle, index: usize) -> Result<(), String> {
    let action = actions::load().into_iter().nth(index).ok_or("no such action")?;
    std::thread::spawn(move || perform(&app, action));
    Ok(())
}

fn perform(app: &AppHandle, action: actions::Action) {
    let hub = app.state::<Hub>();
    let Some((dir, manifest)) = ({
        let mut inner = hub.inner.lock().unwrap();
        inner.action_running = Some(action.name.clone());
        inner.dir.clone().zip(inner.manifest.clone())
    }) else {
        return;
    };
    emit_status(app);
    let before = actions::fingerprint(&dir);
    let result = actions::run(&action, &dir, &manifest);
    {
        let mut inner = hub.inner.lock().unwrap();
        inner.action_running = None;
        // The action may have changed the meeting itself.
        if actions::fingerprint(&dir) != before
            && let Some((_, m)) = meeting::open(&dir)
        {
            inner.manifest = Some(m);
        }
    }
    emit_status(app);
    let _ = app.emit("meeting", ());
    let _ = app.emit(
        "action",
        match result {
            Ok(outcome) => ActionResult {
                message: format!("{}: {}", action.name, outcome.message),
                url: outcome.url,
            },
            Err(why) => ActionResult {
                message: format!("{} failed: {why}", action.name),
                url: None,
            },
        },
    );
}

// ---------------------------------------------------------------------------
// The speech model

#[tauri::command]
fn download_model(app: AppHandle) {
    {
        let hub = app.state::<Hub>();
        let mut inner = hub.inner.lock().unwrap();
        if inner.model_download.is_some() {
            return;
        }
        inner.model_download = Some(0.0);
    }
    emit_status(&app);
    std::thread::spawn(move || {
        let (tx, rx) = async_channel::unbounded::<Event>();
        let reporter = {
            let app = app.clone();
            std::thread::spawn(move || {
                while let Ok(event) = rx.recv_blocking() {
                    match event {
                        Event::Progress(p) => {
                            app.state::<Hub>().inner.lock().unwrap().model_download = Some(p);
                            emit_status(&app);
                        }
                        Event::Finished => break,
                        _ => {}
                    }
                }
            })
        };
        let result = models::ensure(&tx, &Abort::default());
        let _ = tx.send_blocking(Event::Finished);
        let _ = reporter.join();
        app.state::<Hub>().inner.lock().unwrap().model_download = None;
        emit_status(&app);
        match result {
            Ok(_) => toast(&app, "Speech model ready"),
            Err(message) => toast(&app, format!("Could not download the model: {message}")),
        }
    });
}

// ---------------------------------------------------------------------------

/// Sends the live meters to the window while it is open.
fn level_pump(app: AppHandle) {
    std::thread::spawn(move || {
        let mut ticks: u64 = 0;
        loop {
            std::thread::sleep(Duration::from_millis(40));
            ticks += 1;
            let hub = app.state::<Hub>();
            let meter = |levels: Vec<f32>| -> Vec<f32> {
                levels
                    .into_iter()
                    .map(|p| oorapp_capture::to_meter(p) as f32)
                    .collect()
            };
            let elapsed = elapsed(&hub.inner.lock().unwrap());
            let _ = app.emit(
                "levels",
                Levels {
                    mic: meter(hub.mic.levels()),
                    computer: meter(hub.computer.levels()),
                    elapsed,
                },
            );
            // Once a second: the clock in the menu bar, and device changes.
            if ticks.is_multiple_of(25) {
                emit_status(&app);
            }
        }
    });
}

/// Arguments from a second launch, from the file manager, or from a script:
/// `start [name]`, `stop`, `pause`, or a meeting to open.
fn handle_args(app: &AppHandle, args: &[String]) {
    let mut iter = args.iter().skip(1);
    match iter.next().map(String::as_str) {
        Some("start") => {
            let name = iter.cloned().collect::<Vec<_>>().join(" ");
            if let Err(e) = start_recording(app, Some(name)) {
                toast(app, e);
            }
        }
        Some("stop") => stop_recording(app),
        Some("pause") => pause_recording(app),
        Some(path) if !path.starts_with('-') && Path::new(path).exists() => {
            if let Err(e) = open_path(app, Path::new(path)) {
                toast(app, e);
            }
        }
        _ => {}
    }
    show_window(app);
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            handle_args(app, &args);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .manage(Hub::new())
        .manage(notes::Notes::default())
        .on_window_event(|window, event| {
            // During a recording or a transcription the window hides instead
            // of closing; the menu bar item brings it back.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let busy = matches!(
                    window.app_handle().state::<Hub>().inner.lock().unwrap().phase,
                    Phase::Recording | Phase::Saving | Phase::Transcribing
                );
                if busy {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            tray::create(&handle)?;
            notes::register(&handle);
            notes::create_hud(&handle)?;
            updates::start(&handle);
            level_pump(handle.clone());
            let args: Vec<String> = std::env::args().collect();
            handle_args(&handle, &args);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            options,
            status,
            set_format,
            set_language,
            set_title,
            start,
            toggle_pause,
            stop,
            cancel,
            new_recording,
            import_file,
            unfinished,
            recover,
            discard,
            open_meeting,
            meetings,
            meeting_text,
            trash_meeting,
            open_folder,
            copy_text,
            copy_meeting,
            toggle_note,
            set_note_shortcut,
            connect_claude,
            restart_app,
            request_permission,
            updates::check_update,
            updates::install_update,
            meeting_data,
            peaks,
            edit_paragraph,
            delete_paragraph,
            swap_speaker,
            restore_transcript,
            rename_speakers,
            transcribe_again,
            make_chapters,
            run_action,
            download_model,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Oorapp")
        .run(|app, event| {
            // Double-clicking a meeting file in Finder while Oorapp runs.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &event {
                for url in urls {
                    if let Ok(path) = url.to_file_path() {
                        let _ = open_path(app, &path);
                        show_window(app);
                    }
                }
            }
            // Keep running in the menu bar when the window closes during a recording.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                let busy = matches!(
                    app.state::<Hub>().inner.lock().unwrap().phase,
                    Phase::Recording | Phase::Saving | Phase::Transcribing
                );
                if busy && code.is_none() {
                    api.prevent_exit();
                }
            }
            let _ = (app, event);
        });
}
