//! `oorapp` on the command line: everything that works without the app.
//! Transcribing tracks or a file, telling voices apart, asking the agent and
//! running an action. The bench and CI drive Oorapp through this.


use std::process::ExitCode;

use oorapp_core::{APP_NAME, actions, agent, diarize, transcribe};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(2).collect();
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            let backend = if cfg!(feature = "metal") {
                "Metal"
            } else if cfg!(feature = "vulkan") {
                "Vulkan; CPU fallback"
            } else {
                "CPU-only"
            };
            println!("{APP_NAME} {} ({backend})", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("transcribe") => transcribe::cli(&args),
        Some("transcribe-file") => transcribe::cli_file(&args),
        Some("diarize") => diarize::cli(&args),
        Some("ask") => agent::cli(&args),
        Some("action") => actions::cli(&args),
        Some("record") => record(&args),
        Some("mcp") => oorapp_core::mcp::run(),
        Some("devices") => devices(),
        Some("history") => {
            for e in oorapp_core::session::history() {
                println!(
                    "{} | {:7} | {:>5}s | {:>4} words | {} | {}",
                    oorapp_core::local_date(e.started_at),
                    e.kind,
                    e.duration_secs,
                    e.words,
                    e.target_app.as_deref().unwrap_or("-"),
                    e.text.chars().take(70).collect::<String>()
                );
            }
            ExitCode::SUCCESS
        }
        // Not in --help: a file as a quick note would come out, plain text.
        Some("dictate") => {
            let Some(path) = args.first() else { return ExitCode::from(2) };
            let language = args.get(1).map_or("auto", String::as_str);
            let started = std::time::Instant::now();
            let (tx, _rx) = async_channel::unbounded();
            let abort = oorapp_core::transcribe::Abort::default();
            let mut dictation = oorapp_core::transcribe::Dictation::new();
            let result = oorapp_core::transcribe::load_track(std::path::Path::new(path)).and_then(|track| {
                let _ = dictation.warm_up(&tx, &abort);
                eprintln!("model loaded in {:.1}s", started.elapsed().as_secs_f64());
                let t = std::time::Instant::now();
                let out = dictation.transcribe(&track, language, &tx, &abort);
                eprintln!("transcribed in {:.2}s", t.elapsed().as_secs_f64());
                out
            });
            match result {
                Ok(t) => {
                    println!("{}", oorapp_core::transcribe::plain_text(&t));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        // Not in --help: decodes a file to 16 kHz mono f32le on stdout, to
        // compare the decoder with ffmpeg.
        Some("decode") => {
            use std::io::Write;
            let Some(path) = args.first() else { return ExitCode::from(2) };
            let rate = args.get(1).and_then(|r| r.parse().ok()).unwrap_or(16_000);
            match oorapp_core::codec::decode_mono(std::path::Path::new(path), rate) {
                Ok(samples) => {
                    let bytes: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
                    let _ = std::io::stdout().write_all(&bytes);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        None | Some("-h" | "--help") => {
            println!("Usage: {APP_NAME} <command>");
            println!();
            println!("  transcribe <mic> <computer> [--language xx] [--model name]");
            println!("                 transcribe two tracks and print the transcript as Markdown");
            println!("  transcribe-file <audio> [--speakers N] [--language xx] [--model name]");
            println!("                 transcribe one file, telling the voices apart");
            println!("  diarize <audio> [--speakers N]");
            println!("                 only tell the voices apart");
            println!("  ask \"<prompt>\" < text");
            println!("                 run a prompt over stdin through your coding agent, without tools");
            println!("  record <folder> [--seconds N] [--format mono|stereo|separate]");
            println!("                 record mic and computer audio without the app, then export");
            println!("  devices        show which microphone and output are listened to");
            println!("  mcp            serve your meetings and notes to an MCP client over stdio");
            println!("  action \"<name>\" <meeting folder>");
            println!("                 run one of your actions on a meeting");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("{APP_NAME}: unknown command '{other}', see --help");
            ExitCode::from(2)
        }
    }
}

/// `record <folder> [--seconds N]`: both tracks without the app, for trying
/// the capture on a machine. Shows the meters as it goes, Ctrl+C or the
/// seconds end it, then writes the audio and `.tracks` like the app does.
fn record(args: &[String]) -> ExitCode {
    use oorapp_capture::{Kind, Source};
    use oorapp_core::export::{self, Format};
    use std::io::Write;
    use std::time::{Duration, Instant};

    let mut folder = None;
    let mut seconds: Option<u64> = None;
    let mut format = Format::Mono;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--seconds" => seconds = iter.next().and_then(|s| s.parse().ok()),
            "--format" => format = iter.next().map(|f| Format::from_key(f)).unwrap_or(Format::Mono),
            other => folder = Some(std::path::PathBuf::from(other)),
        }
    }
    let Some(folder) = folder else {
        eprintln!("Usage: {APP_NAME} record <folder> [--seconds N] [--format mono|stereo|separate]");
        return ExitCode::from(2);
    };
    let staging = oorapp_core::paths::recordings_dir().join("cli");
    if let Err(e) = std::fs::create_dir_all(&folder).and_then(|()| std::fs::create_dir_all(&staging)) {
        eprintln!("{APP_NAME}: {e}");
        return ExitCode::FAILURE;
    }
    let (mic_raw, system_raw) = (staging.join("mic.raw"), staging.join("system.raw"));
    let (mic, computer) = (Source::spawn(Kind::Mic), Source::spawn(Kind::Computer));
    std::thread::sleep(Duration::from_millis(600));
    for (name, source) in [("mic", &mic), ("computer", &computer)] {
        match (source.device(), source.problem()) {
            (_, Some(problem)) => eprintln!("{name}: {problem}"),
            (Some(device), None) => eprintln!("{name}: {device}"),
            (None, None) => eprintln!("{name}: waiting for the device"),
        }
    }
    if mic.start_recording(&mic_raw).and_then(|()| computer.start_recording(&system_raw)).is_err() {
        eprintln!("{APP_NAME}: could not write to {}", staging.display());
        return ExitCode::FAILURE;
    }
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let stop = stop.clone();
        std::thread::spawn(move || {
            // Enter stops too, for a terminal without Ctrl+C handling.
            // Not at end of input: run with stdin closed, only the seconds end it.
            if std::io::stdin().read_line(&mut String::new()).is_ok_and(|n| n > 0) {
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        });
    }
    let started = Instant::now();
    eprintln!("Recording; press Enter to stop.");
    while !stop.load(std::sync::atomic::Ordering::Relaxed)
        && seconds.is_none_or(|s| started.elapsed().as_secs() < s)
    {
        let bar = |peak: f32| {
            let db = if peak > 0.0 { 20.0 * peak.log10() } else { -60.0 };
            let n = ((db + 60.0) / 60.0 * 30.0).clamp(0.0, 30.0) as usize;
            format!("{:<30}", "#".repeat(n))
        };
        eprint!(
            "\r{:>5.1}s  mic [{}]  computer [{}]",
            started.elapsed().as_secs_f64(),
            bar(mic.recent_peak(5)),
            bar(computer.recent_peak(5))
        );
        let _ = std::io::stderr().flush();
        std::thread::sleep(Duration::from_millis(100));
    }
    mic.stop_recording();
    computer.stop_recording();
    eprintln!();
    let audio = export::export_audio(&mic_raw, &system_raw, &folder, format);
    let tracks = export::export_tracks(&mic_raw, &system_raw, &folder);
    eprintln!(
        "Wrote {} ({}), tracks {}",
        folder.display(),
        if audio { "audio ok" } else { "audio FAILED" },
        if tracks { "ok" } else { "FAILED" }
    );
    if audio && tracks {
        let _ = std::fs::remove_file(&mic_raw);
        let _ = std::fs::remove_file(&system_raw);
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// `devices`: what the two sources open right now.
fn devices() -> ExitCode {
    use oorapp_capture::{Kind, Source};
    let (mic, computer) = (Source::spawn(Kind::Mic), Source::spawn(Kind::Computer));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    for (name, source) in [("mic", &mic), ("computer", &computer)] {
        println!(
            "{name:<9} {}  peak {:.3}{}",
            source.device().unwrap_or_else(|| "-".into()),
            source.recent_peak(50),
            source.problem().map(|p| format!("  ({p})")).unwrap_or_default()
        );
    }
    if computer.digital_silence().as_millis() >= 1000 && cfg!(target_os = "macos") {
        println!();
        println!("The computer audio is exact silence. If something was playing, give the");
        println!("app that runs this (your terminal, or Oorapp) access under System Settings >");
        println!("Privacy & Security > Screen & System Audio Recording > System Audio Recording Only.");
    }
    ExitCode::SUCCESS
}
