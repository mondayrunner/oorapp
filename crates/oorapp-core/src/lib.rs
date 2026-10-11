//! Oorapp's core: everything after the audio is captured, without a GUI.
//! Transcription with whisper.cpp, telling speakers apart, chapters through a
//! coding agent, the meeting folder on disk and your own actions.
//!
//! Ported from Jankees van Woezik's omarchy-meeting-recorder (MIT).

pub mod actions;
pub mod agent;
pub mod chapters;
pub mod codec;
pub mod diarize;
pub mod export;
pub mod mcp;
pub mod meeting;
pub mod models;
pub mod nemotron;
pub mod parakeet;
pub mod paths;
pub mod session;
pub mod settings;
pub mod transcribe;

/// The command and the directory name under the platform's data, config and
/// cache directories.
pub const APP_NAME: &str = "oorapp";
pub const APP_ID: &str = "nl.sitelane.oorapp";

/// The app's own staging format for a track: s16le at this rate and channel
/// count, as it comes from the capture.
pub const RATE: u32 = 48_000;
pub const CHANNELS: u32 = 2;

/// `text` made safe as a file or folder name on macOS, Windows and Linux.
pub fn safe_name(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c| c == ' ' || c == '.');
    if trimmed.is_empty() {
        "Meeting".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// A local time as "2026-10-11 14:30", or "" when it does not exist.
pub fn local_date(unix: i64) -> String {
    jiff::Timestamp::from_second(unix)
        .map(|t| t.to_zoned(jiff::tz::TimeZone::system()).strftime("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}
