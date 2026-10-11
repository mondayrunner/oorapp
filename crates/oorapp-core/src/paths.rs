//! Where Oorapp keeps things, per platform.
//!
//! | | macOS | Windows | Linux |
//! |---|---|---|---|
//! | meetings | `~/Documents/Meetings` | `Documents\Meetings` | `~/Documents/Meetings` |
//! | models | `~/Library/Application Support/oorapp/models` | `%APPDATA%\oorapp\models` | `~/.local/share/oorapp/models` |
//! | config | `~/Library/Application Support/oorapp/config.toml` | `%APPDATA%\oorapp\config.toml` | `~/.config/oorapp/config.toml` |
//! | state | `~/Library/Application Support/oorapp/settings.json` | `%LOCALAPPDATA%\oorapp\settings.json` | `~/.local/state/oorapp/settings.json` |
//! | unfinished recordings | `~/Library/Caches/oorapp/recordings` | `%LOCALAPPDATA%\oorapp\recordings` | `~/.cache/oorapp/recordings` |

use std::path::PathBuf;

use crate::APP_NAME;

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(std::env::temp_dir)
}

pub fn meetings_dir() -> PathBuf {
    dirs::document_dir()
        .unwrap_or_else(|| home().join("Documents"))
        .join("Meetings")
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| home().join(".local/share"))
        .join(APP_NAME)
}

pub fn models_dir() -> PathBuf {
    data_dir().join("models")
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| home().join(".config"))
        .join(APP_NAME)
}

pub fn state_dir() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(|| home().join(".local/state"))
        .join(APP_NAME)
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_NAME)
}

/// Where recordings in progress are written, one folder each, so a crash
/// leaves them behind.
pub fn recordings_dir() -> PathBuf {
    cache_dir().join("recordings")
}

/// The PATH to look up and run agents and actions with. An app started from
/// the Dock or Finder gets launchd's bare PATH, without Homebrew, nvm or
/// `~/.local/bin`, so on macOS and Linux the login shell is asked once for the
/// PATH you have in a terminal, and a few usual places are added after it.
pub fn search_path() -> std::ffi::OsString {
    static PATH: std::sync::OnceLock<std::ffi::OsString> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        #[cfg(unix)]
        {
            if let Some(shell) = login_shell_path() {
                dirs.extend(std::env::split_paths(&shell));
            }
            let home = home();
            dirs.extend([
                home.join(".local/bin"),
                home.join(".claude/local"),
                home.join(".npm-global/bin"),
                home.join(".bun/bin"),
                home.join(".cargo/bin"),
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/local/bin"),
            ]);
        }
        let mut seen = std::collections::HashSet::new();
        dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
        std::env::join_paths(dirs).unwrap_or_default()
    })
    .clone()
}

/// `$PATH` as an interactive login shell sets it, or None after three seconds.
#[cfg(unix)]
fn login_shell_path() -> Option<std::ffi::OsString> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const MARK: &str = "__OORAPP_PATH__";
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/zsh".into());
    let mut child = Command::new(shell)
        .args(["-i", "-l", "-c"])
        .arg(format!("printf '{MARK}%s{MARK}' \"$PATH\""))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < Duration::from_secs(3) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let (_, rest) = out.split_once(MARK)?;
    let (path, _) = rest.split_once(MARK)?;
    Some(path.into())
}
