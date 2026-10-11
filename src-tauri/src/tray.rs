//! The menu-bar item (macOS) or tray icon (Windows): a dot and the time while
//! recording, "paused 01:23" when paused, the progress while transcribing.
//! Clicking it brings the window back; its menu starts and stops.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::{Phase, Status};

const ID: &str = "oorapp";

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Oorapp", true, None::<&str>)?;
    let record = MenuItem::with_id(app, "record", "Start recording", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause", false, None::<&str>)?;
    let note = MenuItem::with_id(app, "note", "New note", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Oorapp", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &PredefinedMenuItem::separator(app)?, &note, &record, &pause, &PredefinedMenuItem::separator(app)?, &quit],
    )?;
    app.manage(TrayItems { record, pause });
    TrayIconBuilder::with_id(ID)
        .icon(tray_icon())
        .icon_as_template(true)
        .tooltip("Oorapp")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => crate::show_window(app),
            "record" => {
                let phase = app.state::<crate::Hub>().status().phase;
                if phase == Phase::Recording {
                    crate::stop_recording(app);
                    crate::show_window(app);
                } else if let Err(e) = crate::start_recording(app, None) {
                    eprintln!("oorapp: {e}");
                }
            }
            "pause" => crate::pause_recording(app),
            "note" => crate::notes::toggle(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::show_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

struct TrayItems {
    record: MenuItem<tauri::Wry>,
    pause: MenuItem<tauri::Wry>,
}

/// Follows the recorder: the title next to the icon (macOS) and the menu.
pub fn update(app: &AppHandle, status: &Status) {
    let Some(tray) = app.tray_by_id(ID) else {
        return;
    };
    let elapsed = clock(status.elapsed);
    let title = match status.phase {
        Phase::Recording if status.paused => Some(format!("paused {elapsed}")),
        Phase::Recording => Some(format!("● {elapsed}")),
        Phase::Saving => Some("saving".to_owned()),
        Phase::Transcribing => Some(format!("{:.0}%", status.progress * 100.0)),
        _ => None,
    };
    // A quick note shows over the rest: it is short and you are holding a key.
    let title = match (status.note, status.note_busy) {
        (Some(secs), _) => Some(format!("● note {}", clock(secs))),
        (None, true) => Some("note…".to_owned()),
        _ => title,
    };
    #[cfg(target_os = "macos")]
    let _ = tray.set_title(title.as_deref());
    let _ = tray.set_tooltip(Some(title.as_deref().map_or("Oorapp".to_owned(), |t| format!("Oorapp: {t}"))));
    if let Some(items) = app.try_state::<TrayItems>() {
        let recording = status.phase == Phase::Recording;
        let _ = items
            .record
            .set_text(if recording { "Stop recording" } else { "Start recording" });
        let _ = items.record.set_enabled(matches!(status.phase, Phase::Idle | Phase::Done | Phase::Recording));
        let _ = items.pause.set_enabled(recording);
        let _ = items.pause.set_text(if status.paused { "Resume" } else { "Pause" });
    }
}

fn clock(secs: i64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// A small two-track waveform as a template image, 22x22 at 2x.
fn tray_icon() -> tauri::image::Image<'static> {
    const SIZE: u32 = 44;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let heights = [4u32, 8, 14, 18, 12, 16, 9, 5];
    let (bar, gap) = (3u32, 2u32);
    let width = heights.len() as u32 * (bar + gap) - gap;
    let x0 = (SIZE - width) / 2;
    let mid = SIZE / 2;
    for (i, h) in heights.iter().enumerate() {
        let x = x0 + i as u32 * (bar + gap);
        let down = (h * 3 / 4).max(3);
        for y in (mid - h)..(mid + down) {
            if y == mid {
                continue;
            }
            for dx in 0..bar {
                let p = ((y * SIZE + x + dx) * 4) as usize;
                rgba[p..p + 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
    }
    tauri::image::Image::new_owned(rgba, SIZE, SIZE)
}
