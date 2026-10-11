//! Quick notes: hold the shortcut, speak, let go, and the text is typed where
//! your cursor is, like Paraspeech. A tap instead of a hold starts a longer
//! note that the next press ends. Only the microphone is recorded; the speech
//! model (Parakeet unless `note_model` says otherwise) stays loaded so a note
//! is text in a moment. Every note is also kept as a meeting folder, so it
//! shows up in the history with its audio.
//!
//! Pasting puts the text on the clipboard and presses ⌘V (Ctrl+V on
//! Windows), then puts back what was on the clipboard. On macOS that needs
//! the Accessibility permission; without it the text stays on the clipboard.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use oorapp_core::meeting;
use oorapp_core::session::{self, RecordingNote};
use oorapp_core::transcribe::{self, Abort, Dictation, Event};
use oorapp_core::{models, paths, settings};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::{Hub, Phase};

/// Holding the right Option key, like dictation apps do: no other app uses
/// it on its own, unlike most key combinations.
pub const DEFAULT_SHORTCUT: &str = "RightAlt";

/// Keys held on their own, by name, with their macOS key code.
const HOLD_KEYS: [(&str, i64, &str); 5] = [
    ("RightAlt", 61, "Right ⌥"),
    ("Fn", 63, "Fn"),
    ("RightCmd", 54, "Right ⌘"),
    ("RightCtrl", 62, "Right ⌃"),
    ("RightShift", 60, "Right ⇧"),
];

/// The choices offered in the app: (value, label).
pub fn choices() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = if cfg!(target_os = "macos") {
        HOLD_KEYS
            .iter()
            .map(|(name, _, label)| ((*name).to_owned(), format!("Hold {label}")))
            .collect()
    } else {
        Vec::new()
    };
    for combo in ["Alt+Shift+Space", "CmdOrCtrl+Shift+Space", "Ctrl+Alt+Space", "F5"] {
        out.push((combo.to_owned(), combo.to_owned()));
    }
    out.push(("none".to_owned(), "Off".to_owned()));
    out
}
/// A press shorter than this is a tap: the note goes on until the next press.
const TAP: Duration = Duration::from_millis(350);
/// Shorter than this is a slip of the finger, not a note.
const MIN_NOTE: Duration = Duration::from_millis(300);

#[derive(Default)]
pub struct Notes {
    state: Mutex<NoteState>,
    dictation: Mutex<Dictation>,
}

#[derive(Default)]
struct NoteState {
    recording: Option<Recording>,
    busy: bool,
}

struct Recording {
    staging: PathBuf,
    started_at: i64,
    pressed: Instant,
    /// Started with a tap: ends on the next press instead of on release.
    toggled: bool,
    /// The app in front when the note started, where the text goes.
    target: Option<Target>,
}

/// The app a note is pasted into.
#[derive(Clone)]
struct Target {
    name: String,
    pid: i32,
}

/// For the status: seconds into a note being spoken, and whether one is
/// being turned into text.
pub fn status(app: &AppHandle) -> (Option<i64>, bool) {
    let notes = app.state::<Notes>();
    let state = notes.state.lock().unwrap();
    (
        state.recording.as_ref().map(|r| r.pressed.elapsed().as_secs() as i64),
        state.busy,
    )
}

/// The key picked in the app, else `note_shortcut` from config.toml, else
/// the default. A held key by name (RightAlt, Fn, ...), a combination such as
/// "Alt+Shift+Space", or "none" for no notes.
pub fn shortcut() -> String {
    settings::load_note_shortcut()
        .or_else(|| models::config_value("note_shortcut"))
        .unwrap_or_else(|| DEFAULT_SHORTCUT.to_owned())
}

/// Which held key is listened for now; a key code, 0 for none. The event tap
/// keeps running and reads this, so a new choice works at once.
static HOLD_KEY: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// Listens for the note key as it is set now.
pub fn apply_shortcut(app: &AppHandle) -> Result<(), String> {
    let shortcut = shortcut();
    let _ = app.global_shortcut().unregister_all();
    HOLD_KEY.store(0, std::sync::atomic::Ordering::Relaxed);
    if shortcut.eq_ignore_ascii_case("none") {
        return Ok(());
    }
    if let Some((_, code, _)) = HOLD_KEYS.iter().find(|(name, _, _)| name.eq_ignore_ascii_case(&shortcut)) {
        HOLD_KEY.store(*code, std::sync::atomic::Ordering::Relaxed);
        start_key_listener(app);
        return Ok(());
    }
    app.global_shortcut()
        .on_shortcut(shortcut.as_str(), |app, _, event| match event.state() {
            ShortcutState::Pressed => pressed(app),
            ShortcutState::Released => released(app),
        })
        .map_err(|e| format!("{shortcut} cannot be used: {e}"))
}

pub fn register(app: &AppHandle) {
    if let Err(e) = apply_shortcut(app) {
        eprintln!("oorapp: notes: {e}");
    }
    // Load the speech model now when it is already on disk, so the first
    // note does not wait for it.
    let app = app.clone();
    std::thread::spawn(move || {
        let ready = match models::config_value("note_model") {
            Some(name) if !oorapp_core::parakeet::is_parakeet(&name) => models::find().is_some(),
            _ => oorapp_core::parakeet::find().is_some(),
        };
        if ready {
            let (tx, _rx) = async_channel::unbounded::<Event>();
            let notes = app.state::<Notes>();
            let _ = notes.dictation.lock().unwrap().warm_up(&tx, &Abort::default());
        }
    });
}

fn pressed(app: &AppHandle) {
    let notes = app.state::<Notes>();
    let state = notes.state.lock().unwrap();
    match state.recording.as_ref() {
        Some(r) if r.toggled => {
            drop(state);
            stop(app);
        }
        Some(_) => {} // still held
        None if state.busy => {}
        None => {
            drop(state);
            start(app);
        }
    }
}

fn released(app: &AppHandle) {
    let notes = app.state::<Notes>();
    let mut state = notes.state.lock().unwrap();
    let Some(r) = state.recording.as_mut() else {
        return;
    };
    if r.toggled {
        return;
    }
    if r.pressed.elapsed() < TAP {
        r.toggled = true;
        drop(state);
        crate::emit_status(app);
    } else {
        drop(state);
        stop(app);
    }
}

/// A held key that turned out to be part of typing (⌥ with a letter): the
/// note is thrown away.
fn cancel(app: &AppHandle) {
    let notes = app.state::<Notes>();
    let Some(recording) = notes.state.lock().unwrap().recording.take() else {
        return;
    };
    app.state::<Hub>().mic.stop_recording();
    let _ = std::fs::remove_dir_all(&recording.staging);
    hide_hud(app);
    crate::emit_status(app);
}

/// One event tap for the life of the app, on a thread of its own: it hears
/// the held key go down and up, and any other key in between. It needs the
/// Accessibility permission, the same one pasting needs; until that is given
/// it tries again every few seconds.
#[cfg(target_os = "macos")]
fn start_key_listener(app: &AppHandle) {
    use core_foundation::runloop::CFRunLoop;
    use core_graphics::event::{
        CallbackResult, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
        EventField,
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let app = app.clone();
    std::thread::Builder::new()
        .name("note-key".into())
        .spawn(move || {
            let mut asked = false;
            loop {
                // Without Input Monitoring a tap only hears keys meant for
                // Oorapp itself, so it waits for the permission first.
                if !may_listen() {
                    if !asked {
                        asked = true;
                        request_listen();
                    }
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
                let held = AtomicBool::new(false);
                let typed = AtomicBool::new(false);
                let tap_app = app.clone();
                let result = CGEventTap::with_enabled(
                    CGEventTapLocation::Session,
                    CGEventTapPlacement::HeadInsertEventTap,
                    CGEventTapOptions::ListenOnly,
                    vec![CGEventType::FlagsChanged, CGEventType::KeyDown],
                    |_, kind, event| {
                        let want = HOLD_KEY.load(Ordering::Relaxed);
                        if want == 0 {
                            return CallbackResult::Keep;
                        }
                        match kind {
                            CGEventType::FlagsChanged => {
                                let code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
                                if code != want {
                                    return CallbackResult::Keep;
                                }
                                let was = held.load(Ordering::Relaxed);
                                // Pressing and letting go of a modifier both
                                // arrive as a flags change; the key's own
                                // state flips each time.
                                if !was {
                                    held.store(true, Ordering::Relaxed);
                                    typed.store(false, Ordering::Relaxed);
                                    pressed(&tap_app);
                                } else {
                                    held.store(false, Ordering::Relaxed);
                                    if typed.load(Ordering::Relaxed) {
                                        cancel(&tap_app);
                                    } else {
                                        released(&tap_app);
                                    }
                                }
                            }
                            CGEventType::KeyDown if held.load(Ordering::Relaxed) => {
                                typed.store(true, Ordering::Relaxed);
                            }
                            _ => {}
                        }
                        CallbackResult::Keep
                    },
                    CFRunLoop::run_current,
                );
                if result.is_err() {
                    eprintln!("oorapp: could not listen for the note key");
                }
                std::thread::sleep(Duration::from_secs(3));
            }
        })
        .expect("note key thread");
}

#[cfg(not(target_os = "macos"))]
fn start_key_listener(app: &AppHandle) {
    crate::toast(app, "Holding a single key is for macOS; pick a key combination");
}

fn start(app: &AppHandle) {
    let hub = app.state::<Hub>();
    if hub.inner.lock().unwrap().phase == Phase::Recording {
        crate::toast(app, "A meeting is being recorded; notes wait until it is done");
        return;
    }
    let started_at = session::now();
    let staging = paths::recordings_dir().join(format!("note-{started_at}"));
    let mic = session::mic_raw(&staging);
    if let Err(e) = std::fs::create_dir_all(&staging)
        .and_then(|()| std::fs::write(session::system_raw(&staging), []))
        .and_then(|()| hub.mic.start_recording(&mic))
    {
        crate::toast(app, format!("Could not start a note: {e}"));
        return;
    }
    session::write_recording_note(&staging, &note_for(started_at));
    let notes = app.state::<Notes>();
    notes.state.lock().unwrap().recording = Some(Recording {
        staging,
        started_at,
        pressed: Instant::now(),
        toggled: false,
        target: frontmost_app(),
    });
    crate::emit_status(app);
    show_hud(app);
}

/// Starts a note, or ends the one that runs: the button in the window and
/// the menu bar. It is a tap, so it runs until the next one.
pub fn toggle(app: &AppHandle) {
    let notes = app.state::<Notes>();
    let running = notes.state.lock().unwrap().recording.is_some();
    if running {
        stop(app);
    } else {
        start(app);
        if let Some(r) = notes.state.lock().unwrap().recording.as_mut() {
            r.toggled = true;
        }
        crate::emit_status(app);
    }
}

pub const HUD: &str = "hud";

/// The small pill at the bottom of the screen while a note is spoken and
/// turned into text, like Paraspeech's. It never takes the focus.
pub fn create_hud(app: &AppHandle) -> tauri::Result<()> {
    let (width, height) = (240.0, 52.0);
    let mut builder = tauri::WebviewWindowBuilder::new(app, HUD, tauri::WebviewUrl::App("index.html".into()))
        .title("Oorapp note")
        .inner_size(width, height)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .visible(false);
    if let Some(monitor) = app.primary_monitor()? {
        let scale = monitor.scale_factor();
        let size = monitor.size().to_logical::<f64>(scale);
        let origin = monitor.position().to_logical::<f64>(scale);
        builder = builder.position(
            origin.x + (size.width - width) / 2.0,
            origin.y + size.height - height - 90.0,
        );
    }
    builder.build()?;
    Ok(())
}

fn show_hud(app: &AppHandle) {
    if let Some(hud) = app.get_webview_window(HUD) {
        let _ = hud.show();
    }
}

fn hide_hud(app: &AppHandle) {
    if let Some(hud) = app.get_webview_window(HUD) {
        let _ = hud.hide();
    }
}

fn note_for(started_at: i64) -> RecordingNote {
    RecordingNote {
        title: format!("Note {}", jiff::Zoned::now().strftime("%H:%M")),
        started_at,
        format: oorapp_core::export::Format::Mono,
        language: settings::load_language().to_owned(),
    }
}

fn stop(app: &AppHandle) {
    let notes = app.state::<Notes>();
    let Some(recording) = ({
        let mut state = notes.state.lock().unwrap();
        let r = state.recording.take();
        state.busy = r.is_some();
        r
    }) else {
        return;
    };
    app.state::<Hub>().mic.stop_recording();
    crate::emit_status(app);
    let app = app.clone();
    std::thread::spawn(move || {
        let result = finish(&app, &recording);
        app.state::<Notes>().state.lock().unwrap().busy = false;
        crate::emit_status(&app);
        hide_hud(&app);
        match result {
            Ok(Some(true)) => crate::toast(&app, "Note pasted"),
            Ok(Some(false)) => crate::toast(&app, "Note copied; paste it with ⌘V"),
            Ok(None) => {}
            Err(e) => crate::toast(&app, format!("The note did not work: {e}")),
        }
    });
}

/// Turns the note into text, pastes it and keeps it. Some(pasted) when there
/// was something to paste.
fn finish(app: &AppHandle, recording: &Recording) -> Result<Option<bool>, String> {
    let staging = &recording.staging;
    if recording.pressed.elapsed() < MIN_NOTE + Duration::from_millis(50) && !recording.toggled {
        let _ = std::fs::remove_dir_all(staging);
        return Ok(None);
    }
    let track = transcribe::load_track(&session::mic_raw(staging))?;
    let note = session::read_recording_note(staging).unwrap_or_else(|| note_for(recording.started_at));
    if oorapp_core::parakeet::find().is_none() && models::config_value("note_model").is_none() {
        crate::toast(app, "Downloading the speech model for notes (670 MB), once");
    }
    let (tx, _rx) = async_channel::unbounded::<Event>();
    let transcript = app
        .state::<Notes>()
        .dictation
        .lock()
        .unwrap()
        .transcribe(&track, &note.language, &tx, &Abort::default())?;
    let text = transcribe::plain_text(&transcript);
    if text.is_empty() {
        let _ = std::fs::remove_dir_all(staging);
        crate::toast(app, "Nothing heard");
        return Ok(None);
    }
    // Started from Oorapp's own window: nothing to paste into, so it is copied.
    let target = recording
        .target
        .clone()
        .filter(|t| t.pid != std::process::id() as i32);
    let pasted = match &target {
        Some(target) => paste(app, &text, target),
        None => {
            let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text.clone()));
            false
        }
    };

    // Keep it like a meeting: the audio, the transcript and the manifest.
    let out = session::free_output_dir(note.started_at, &note.title);
    let (audio, _tracks, mut manifest) = session::save_recording(staging, &out, &note);
    let you = settings::load_your_name();
    manifest.kind = Some("note".to_owned());
    manifest.target_app = target.map(|t| t.name);
    manifest.speakers = vec![you.clone()];
    manifest.labels = vec![meeting::DEFAULT_YOU.to_owned()];
    manifest.language = transcript.language.clone();
    manifest.model = Some(
        models::config_value("note_model").unwrap_or_else(|| oorapp_core::parakeet::NAME.to_owned()),
    );
    let markdown = transcribe::to_markdown(&note.title, &oorapp_core::local_date(note.started_at), &transcript);
    let markdown = meeting::relabel_all(&markdown, &[(meeting::DEFAULT_YOU.to_owned(), you)]);
    let _ = meeting::write(&out, &manifest);
    let _ = std::fs::write(out.join("transcript.md"), markdown);
    if audio {
        let _ = std::fs::remove_dir_all(staging);
    }
    Ok(Some(pasted))
}

/// Pastes `text` where the cursor is and puts back the clipboard. False when
/// it could only copy.
fn paste(app: &AppHandle, text: &str, target: &Target) -> bool {
    let Ok(mut clipboard) = arboard::Clipboard::new() else {
        return false;
    };
    let previous = clipboard.get_text().ok();
    if clipboard.set_text(text).is_err() {
        return false;
    }
    if !may_type() {
        ask_for_accessibility(app);
        return false;
    }
    // Back to the app the note was spoken for, in case anything took the focus.
    activate(target);
    std::thread::sleep(Duration::from_millis(120));
    if press_paste().is_err() {
        return false;
    }
    // Long enough for the other app to read the clipboard.
    std::thread::sleep(Duration::from_millis(500));
    if let Some(previous) = previous {
        let _ = clipboard.set_text(previous);
    }
    true
}

/// ⌘V on macOS, posted straight to the system with the key code of V, so
/// it works from any thread. (Asking for the key of a letter goes through the
/// keyboard layout, which macOS only allows on the main thread.)
#[cfg(target_os = "macos")]
fn press_paste() -> Result<(), String> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    const KEY_V: u16 = 9; // kVK_ANSI_V
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|()| "no event source".to_string())?;
    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(source.clone(), KEY_V, down)
            .map_err(|()| "could not make the key event".to_string())?;
        event.set_flags(CGEventFlags::CGEventFlagCommand);
        event.post(CGEventTapLocation::HID);
    }
    Ok(())
}

/// Ctrl+V elsewhere, by virtual key code.
#[cfg(not(target_os = "macos"))]
fn press_paste() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    enigo.key(Key::Control, Direction::Press).map_err(|e| e.to_string())?;
    let typed = enigo.key(Key::Other(0x56), Direction::Click); // VK_V
    enigo.key(Key::Control, Direction::Release).map_err(|e| e.to_string())?;
    typed.map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn may_type() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    // SAFETY: a plain query without arguments.
    unsafe { AXIsProcessTrusted() }
}

#[cfg(not(target_os = "macos"))]
fn may_type() -> bool {
    true
}

/// Once per run: opens the Accessibility settings so Oorapp can be allowed.
fn ask_for_accessibility(app: &AppHandle) {
    static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if ASKED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    crate::toast(app, "To paste for you, Oorapp needs Accessibility access");
    #[cfg(target_os = "macos")]
    {
        use tauri_plugin_opener::OpenerExt;
        let _ = app.opener().open_url(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            None::<&str>,
        );
    }
}

/// The app in front, such as Ghostty.
#[cfg(target_os = "macos")]
fn frontmost_app() -> Option<Target> {
    let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
    let app = workspace.frontmostApplication()?;
    Some(Target {
        name: app.localizedName()?.to_string(),
        pid: app.processIdentifier(),
    })
}

#[cfg(target_os = "macos")]
fn activate(target: &Target) {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(target.pid) {
        #[allow(deprecated)]
        app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
    }
}

#[cfg(not(target_os = "macos"))]
fn frontmost_app() -> Option<Target> {
    None
}

#[cfg(not(target_os = "macos"))]
fn activate(_: &Target) {}

/// What notes need from macOS: hearing the key outside Oorapp (Input
/// Monitoring) and pasting into another app (Accessibility).
#[derive(Clone, Copy, serde::Serialize)]
pub struct Permissions {
    pub keys: bool,
    pub paste: bool,
}

pub fn permissions() -> Permissions {
    Permissions {
        keys: !uses_held_key() || may_listen(),
        paste: may_type(),
    }
}

fn uses_held_key() -> bool {
    HOLD_KEY.load(std::sync::atomic::Ordering::Relaxed) != 0
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

#[cfg(target_os = "macos")]
fn may_listen() -> bool {
    // SAFETY: plain queries without arguments.
    unsafe { CGPreflightListenEventAccess() }
}

#[cfg(target_os = "macos")]
fn request_listen() {
    // SAFETY: shows the system prompt once; no arguments.
    unsafe {
        CGRequestListenEventAccess();
    }
}

#[cfg(not(target_os = "macos"))]
fn may_listen() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
fn request_listen() {}

/// Asks macOS for a permission and opens its page in System Settings:
/// "keys" for Input Monitoring, "paste" for Accessibility.
pub fn request_permission(app: &AppHandle, which: &str) {
    #[cfg(target_os = "macos")]
    {
        use tauri_plugin_opener::OpenerExt;
        let pane = if which == "keys" {
            request_listen();
            "Privacy_ListenEvent"
        } else {
            prompt_accessibility();
            "Privacy_Accessibility"
        };
        let _ = app.opener().open_url(
            format!("x-apple.systempreferences:com.apple.preference.security?{pane}"),
            None::<&str>,
        );
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, which);
}

/// The system prompt that adds Oorapp to the Accessibility list.
#[cfg(target_os = "macos")]
fn prompt_accessibility() {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrustedWithOptions(options: core_foundation::dictionary::CFDictionaryRef) -> bool;
    }
    let options = CFDictionary::from_CFType_pairs(&[(
        CFString::from_static_string("AXTrustedCheckOptionPrompt"),
        CFBoolean::true_value(),
    )]);
    // SAFETY: a valid dictionary that lives across the call.
    unsafe {
        AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef());
    }
}
