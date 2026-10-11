//! Updates from GitHub Releases: checked at start and every six hours. When
//! there is one, the window shows it; installing downloads it, checks the
//! signature against the public key in tauri.conf.json, and restarts.
//! Never during a recording or a transcription.

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::{Hub, Phase};

#[derive(Clone, Serialize)]
pub struct Available {
    version: String,
    notes: Option<String>,
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Let the window settle first.
        tokio_sleep(Duration::from_secs(20)).await;
        loop {
            if let Some(available) = check(&app).await {
                let _ = app.emit("update", available);
            }
            tokio_sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}

async fn tokio_sleep(duration: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(duration)).await;
}

async fn check(app: &AppHandle) -> Option<Available> {
    let update = app.updater().ok()?.check().await.ok()??;
    Some(Available {
        version: update.version.clone(),
        notes: update.body.clone(),
    })
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Option<Available> {
    check(&app).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let busy = matches!(
        app.state::<Hub>().inner.lock().unwrap().phase,
        Phase::Recording | Phase::Saving | Phase::Transcribing
    );
    if busy {
        return Err("Finish the recording first; the update restarts Oorapp".into());
    }
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Oorapp is up to date")?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}
