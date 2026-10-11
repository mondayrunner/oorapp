//! `oorapp mcp`: Oorapp as an MCP server over stdio, so Claude Code and other
//! MCP clients can read your meetings and notes, search them, and transcribe
//! a file. JSON-RPC 2.0, one message per line; logs go to stderr only.
//!
//! Everything stays on this computer: the server reads the meeting folders in
//! `~/Documents/Meetings` and never records (the app owns the microphone).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::meeting::{self, Manifest};
use crate::session::{self, HistoryEntry, Tracks};
use crate::transcribe::{Abort, Event};
use crate::{APP_NAME, paths};
use serde_json::{Value, json};

const PROTOCOL: &str = "2025-06-18";
const DEFAULT_LIMIT: usize = 20;
const SNIPPET: usize = 160;

pub fn run() -> std::process::ExitCode {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(&message),
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(reply) = reply {
            let _ = writeln!(stdout, "{reply}");
            let _ = stdout.flush();
        }
    }
    std::process::ExitCode::SUCCESS
}

/// The reply to one message; None for a notification.
fn handle(message: &Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message["method"].as_str().unwrap_or("");
    let id = id?; // notifications (initialized, cancelled) get no reply
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": message["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": APP_NAME, "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Oorapp's meetings and quick notes, transcribed on this computer. \
                Use search or list_meetings to find one, then get_transcript with its path.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = message["params"]["name"].as_str().unwrap_or("");
            let args = &message["params"]["arguments"];
            Ok(match call(name, args) {
                Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
                Err(why) => json!({ "content": [{ "type": "text", "text": why }], "isError": true }),
            })
        }
        other => Err((-32601, format!("method not found: {other}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, why)) => error(id, code, &why),
    })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn tools() -> Value {
    let kind = json!({ "type": "string", "enum": ["note", "meeting", "file"],
        "description": "note: a quick spoken note; meeting: a recorded call; file: an imported audio file" });
    json!([
        {
            "name": "list_meetings",
            "description": "Meetings, notes and imported files in Oorapp, newest first.",
            "inputSchema": { "type": "object", "properties": {
                "kind": kind,
                "limit": { "type": "integer", "description": "At most this many (default 20)" },
                "since": { "type": "string", "description": "Only from this date on, YYYY-MM-DD" }
            } },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "search",
            "description": "Searches titles and transcripts, case-insensitive; returns matches with a snippet around the hit.",
            "inputSchema": { "type": "object", "required": ["query"], "properties": {
                "query": { "type": "string" },
                "kind": kind,
                "limit": { "type": "integer", "description": "At most this many (default 20)" }
            } },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "get_transcript",
            "description": "The transcript of a meeting (Markdown, speaker and time per line) or the text of a note, with the speakers and chapters. `path` is a meeting folder or its .oorapp file, as list_meetings and search return it.",
            "inputSchema": { "type": "object", "required": ["path"], "properties": {
                "path": { "type": "string" }
            } },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "get_latest",
            "description": "The newest meeting, note or file, with its transcript.",
            "inputSchema": { "type": "object", "properties": { "kind": kind } },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "transcribe_file",
            "description": "Transcribes an audio or video file on this computer, telling the voices apart, and keeps it as a meeting in Oorapp. Takes about a tenth of the file's length, minutes for a long file. Returns the transcript.",
            "inputSchema": { "type": "object", "required": ["path"], "properties": {
                "path": { "type": "string", "description": "Absolute path to the file" },
                "language": { "type": "string", "description": "auto (default), en, nl, de, fr, es, it or pt" },
                "speakers": { "type": "integer", "description": "How many people speak; leave out to find it automatically" }
            } }
        }
    ])
}

fn call(name: &str, args: &Value) -> Result<String, String> {
    match name {
        "list_meetings" => {
            let since = match args["since"].as_str() {
                Some(date) => Some(start_of_day(date)?),
                None => None,
            };
            let entries: Vec<Value> = history(args["kind"].as_str())
                .into_iter()
                .filter(|e| since.is_none_or(|s| e.started_at >= s))
                .take(limit(args))
                .map(|e| summary(&e))
                .collect();
            pretty(&json!({ "count": entries.len(), "meetings": entries }))
        }
        "search" => {
            let query = args["query"].as_str().map(str::trim).filter(|q| !q.is_empty()).ok_or("query is required")?;
            let needle = query.to_lowercase();
            let mut hits = Vec::new();
            for entry in history(args["kind"].as_str()) {
                let text = full_text(&entry);
                let in_title = entry.title.to_lowercase().contains(&needle);
                let at = text.to_lowercase().find(&needle);
                if !in_title && at.is_none() {
                    continue;
                }
                let mut hit = summary(&entry);
                hit["snippet"] = json!(match at {
                    Some(at) => snippet(&text, at, needle.len()),
                    None => text.chars().take(SNIPPET).collect(),
                });
                hits.push(hit);
                if hits.len() >= limit(args) {
                    break;
                }
            }
            pretty(&json!({ "query": query, "count": hits.len(), "results": hits }))
        }
        "get_transcript" => {
            let path = args["path"].as_str().ok_or("path is required")?;
            let (dir, manifest) = open_meeting(path)?;
            transcript(&dir, &manifest)
        }
        "get_latest" => {
            let entry = history(args["kind"].as_str()).into_iter().next().ok_or("Nothing in Oorapp yet")?;
            let (dir, manifest) = meeting::open(&entry.path).ok_or("The newest meeting cannot be read")?;
            transcript(&dir, &manifest)
        }
        "transcribe_file" => transcribe_file(args),
        other => Err(format!("unknown tool: {other}")),
    }
}

fn history(kind: Option<&str>) -> Vec<HistoryEntry> {
    session::history()
        .into_iter()
        .filter(|e| kind.is_none_or(|k| e.kind == k))
        .collect()
}

fn limit(args: &Value) -> usize {
    args["limit"].as_u64().map_or(DEFAULT_LIMIT, |n| n.clamp(1, 500) as usize)
}

fn summary(e: &HistoryEntry) -> Value {
    json!({
        "title": e.title,
        "date": crate::local_date(e.started_at),
        "duration": clock(e.duration_secs),
        "kind": e.kind,
        "path": e.path.display().to_string(),
        "words": e.words,
        "transcribed": e.transcribed,
        "target_app": e.target_app,
    })
}

/// The searchable text: a note's text, a meeting's whole transcript.
fn full_text(e: &HistoryEntry) -> String {
    if e.kind == "note" {
        return e.text.clone();
    }
    std::fs::read_to_string(e.path.join("transcript.md"))
        .map(|md| {
            session::View::parse(&md)
                .paragraphs
                .iter()
                .map(|p| format!("{}: {}", p.speaker, p.text))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// About `SNIPPET` characters around the byte offset `at` in `text`.
fn snippet(text: &str, at: usize, len: usize) -> String {
    let before = text[..at].chars().rev().take(SNIPPET / 2).collect::<Vec<_>>();
    let start = at - before.iter().map(|c| c.len_utf8()).sum::<usize>();
    let end = text[at..]
        .char_indices()
        .map(|(i, _)| at + i)
        .find(|&i| i >= at + len + SNIPPET / 2)
        .unwrap_or(text.len());
    let mut out = text[start..end].replace('\n', " ");
    if start > 0 {
        out.insert(0, '…');
    }
    if end < text.len() {
        out.push('…');
    }
    out
}

fn transcript(dir: &Path, manifest: &Manifest) -> Result<String, String> {
    let entry = session::history_entry(dir, manifest);
    let body = if entry.kind == "note" {
        entry.text.clone()
    } else {
        std::fs::read_to_string(dir.join("transcript.md"))
            .map_err(|_| "This meeting has no transcript yet".to_owned())?
    };
    let chapters: Vec<Value> = manifest
        .chapters
        .iter()
        .map(|c| json!({ "at": crate::chapters::clock(c.start_ms), "title": c.title }))
        .collect();
    let meta = json!({
        "title": manifest.title,
        "date": crate::local_date(manifest.started_at),
        "duration": clock(manifest.duration_secs),
        "kind": entry.kind,
        "language": manifest.language,
        "speakers": manifest.people(),
        "chapters": chapters,
        "path": dir.display().to_string(),
        "target_app": manifest.target_app,
    });
    Ok(format!("{}\n\n{body}", serde_json::to_string_pretty(&meta).unwrap_or_default()))
}

/// A meeting folder or its manifest, only inside the meetings folder.
fn open_meeting(path: &str) -> Result<(PathBuf, Manifest), String> {
    let path = Path::new(path);
    let path = path.canonicalize().map_err(|_| format!("{} does not exist", path.display()))?;
    let root = paths::meetings_dir()
        .canonicalize()
        .map_err(|_| "There is no meetings folder yet".to_owned())?;
    if !path.starts_with(&root) {
        return Err(format!("Only meetings in {} can be read", root.display()));
    }
    meeting::open(&path).ok_or_else(|| "This is not an Oorapp meeting".to_owned())
}

fn transcribe_file(args: &Value) -> Result<String, String> {
    let source = PathBuf::from(args["path"].as_str().ok_or("path is required")?);
    if !source.is_file() {
        return Err(format!("{} is not a file", source.display()));
    }
    let language = args["language"].as_str().unwrap_or("auto");
    if !crate::transcribe::LANGUAGES.iter().any(|(c, _)| *c == language) {
        return Err(format!("unknown language {language}"));
    }
    let speakers = args["speakers"].as_u64().filter(|n| *n > 0).map(|n| n as usize);
    let (out, mut manifest) = session::import_manifest(&source, language, speakers);
    eprintln!("{APP_NAME} mcp: transcribing {} into {}", source.display(), out.display());
    manifest.duration_secs = session::import_audio(&source, &out)?;
    let _ = meeting::write(&out, &manifest);
    // Progress goes to stderr, so a client's log shows it is still working.
    let (tx, rx) = async_channel::unbounded::<Event>();
    let reporter = std::thread::spawn(move || {
        let mut last = String::new();
        while let Ok(event) = rx.recv_blocking() {
            match event {
                Event::Stage(stage) if stage != last => {
                    eprintln!("{APP_NAME} mcp: {stage}");
                    last = stage;
                }
                Event::Finished => break,
                _ => {}
            }
        }
    });
    let result = session::transcribe_into(
        &out,
        &Tracks::Single(session::source_track(&out), speakers),
        language,
        &mut manifest,
        &tx,
        &Abort::default(),
    );
    let _ = tx.send_blocking(Event::Finished);
    let _ = reporter.join();
    result?;
    transcript(&out, &manifest)
}

fn start_of_day(date: &str) -> Result<i64, String> {
    let day: jiff::civil::Date = date.parse().map_err(|_| format!("since must be YYYY-MM-DD, not {date}"))?;
    day.to_zoned(jiff::tz::TimeZone::system())
        .map(|z| z.timestamp().as_second())
        .map_err(|e| e.to_string())
}

fn clock(secs: i64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

fn pretty(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_get_no_reply_and_unknown_methods_an_error() {
        assert!(handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
        let reply = handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "nope" })).unwrap();
        assert_eq!(reply["error"]["code"], -32601);
    }

    #[test]
    fn snippets_cut_on_characters_around_the_hit() {
        let text = "é".repeat(200) + "needle" + &"ü".repeat(200);
        let at = text.find("needle").unwrap();
        let s = snippet(&text, at, 6);
        assert!(s.contains("needle") && s.starts_with('…') && s.ends_with('…'));
    }

    #[test]
    fn paths_outside_the_meetings_folder_are_refused() {
        assert!(open_meeting("/etc").is_err());
    }
}
