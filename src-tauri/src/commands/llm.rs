//! DotFlow — commands for the optional local-LLM picker (the curated catalog behind the review
//! overlay's Rewrite / Formal / Summarize actions). These are pure file + catalog + settings
//! management and are intentionally **feature-independent** (no `local-llm` cargo feature): the picker
//! works in default builds so a user can download and select a model; only the actual inference in
//! [`crate::dotflow::local_llm`] needs the feature.
//!
//! The integration point is deliberately minimal: [`select_llm_model`] just sets
//! `settings.local_llm_model_path` to the chosen model's downloaded file, which is exactly what
//! `ai_transform` / `ai_transform_available` already read — so the AI path works unchanged.
//!
//! Download reuses the STT model download *pattern* (`reqwest` streaming to a `.partial` file, resume
//! via HTTP Range, throttled progress events, size verification on completion, atomic rename). It emits
//! `llm-download-progress` (payload: the shared [`DownloadProgress`] shape), `llm-download-complete`,
//! and `llm-download-failed` so the UI can show a progress bar — mirroring the STT
//! `model-download-*` events but on a distinct channel so the two pickers never cross-talk.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use log::{info, warn};
use tauri::{AppHandle, Emitter};

use crate::dotflow::llm_catalog::{self, LlmModelInfo};
use crate::managers::model::DownloadProgress;
use crate::settings::{get_settings, write_settings};

/// Process-wide set of model ids with an in-flight download, so a second `download_llm_model` for the
/// same id is rejected instead of racing the first onto the same `.partial` file.
fn in_flight() -> &'static Mutex<HashSet<String>> {
    static S: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashSet::new()))
}

/// RAII guard that removes a model id from the in-flight set on every exit path.
struct InFlightGuard(String);
impl Drop for InFlightGuard {
    fn drop(&mut self) {
        in_flight().lock().unwrap().remove(&self.0);
    }
}

/// List the curated LLM catalog with live on-disk status. `downloaded` = the GGUF exists in the llm dir
/// with the exact catalog size; `active` = its path equals `settings.local_llm_model_path`.
#[tauri::command]
#[specta::specta]
pub fn list_llm_models(app: AppHandle) -> Vec<LlmModelInfo> {
    let dir = llm_catalog::llm_dir(&app);
    let active_path = get_settings(&app).local_llm_model_path.trim().to_string();
    let active = std::path::PathBuf::from(&active_path);

    llm_catalog::catalog()
        .into_iter()
        .map(|mut m| {
            let path = dir.join(&m.filename);
            m.downloaded = llm_catalog::file_is_complete(&path, m.size_bytes);
            m.active = !active_path.is_empty() && active == path;
            m
        })
        .collect()
}

/// Download a catalog model into the llm dir with progress events. No-op if already complete. Verifies
/// the finished file's size matches the catalog before renaming it into place. Reuses the STT download
/// pattern (`.partial` + Range resume + throttled `llm-download-progress`).
#[tauri::command]
#[specta::specta]
pub async fn download_llm_model(app: AppHandle, id: String) -> Result<(), String> {
    let result = download_llm_model_inner(&app, &id).await;
    match &result {
        Ok(()) => {
            let _ = app.emit("llm-download-complete", &id);
        }
        Err(error) => {
            let _ = app.emit(
                "llm-download-failed",
                serde_json::json!({ "model_id": &id, "error": error }),
            );
        }
    }
    result
}

async fn download_llm_model_inner(app: &AppHandle, id: &str) -> Result<(), String> {
    let info = llm_catalog::find(id).ok_or_else(|| format!("Unknown LLM model: {id}"))?;

    let dir = llm_catalog::llm_dir(app);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create llm dir: {e}"))?;

    let final_path = dir.join(&info.filename);
    let partial_path = dir.join(format!("{}.partial", info.filename));

    // Already downloaded — clean up any stray partial and return.
    if llm_catalog::file_is_complete(&final_path, info.size_bytes) {
        if partial_path.exists() {
            let _ = fs::remove_file(&partial_path);
        }
        return Ok(());
    }

    // Single-flight per id.
    {
        let mut set = in_flight().lock().unwrap();
        if !set.insert(id.to_string()) {
            return Err(format!("Download already in progress: {id}"));
        }
    }
    let _guard = InFlightGuard(id.to_string());

    // Resume support: if a partial exists, continue from where it left off.
    let mut resume_from = if partial_path.exists() {
        partial_path.metadata().map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    if resume_from >= info.size_bytes && info.size_bytes > 0 {
        // Partial already at/over the expected size but not verified as final — start fresh.
        let _ = fs::remove_file(&partial_path);
        resume_from = 0;
    }

    let client = reqwest::Client::new();
    let mut request = client.get(&info.url);
    if resume_from > 0 {
        info!("Resuming LLM download {id} from byte {resume_from}");
        request = request.header("Range", format!("bytes={resume_from}-"));
    } else {
        info!("Starting LLM download {id} from {}", info.url);
    }

    let mut response = request
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    // Server ignored the Range (200 instead of 206): restart fresh to avoid corrupting the file.
    if resume_from > 0 && response.status() == reqwest::StatusCode::OK {
        warn!("Server ignored Range for {id}; restarting download");
        drop(response);
        let _ = fs::remove_file(&partial_path);
        resume_from = 0;
        response = client
            .get(&info.url)
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;
    }

    if !response.status().is_success() && response.status() != reqwest::StatusCode::PARTIAL_CONTENT
    {
        return Err(format!("Download failed: HTTP {}", response.status()));
    }

    let total = if resume_from > 0 {
        resume_from + response.content_length().unwrap_or(0)
    } else {
        response.content_length().unwrap_or(info.size_bytes)
    };

    let mut file = if resume_from > 0 {
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&partial_path)
    } else {
        fs::File::create(&partial_path)
    }
    .map_err(|e| format!("Failed to open partial file: {e}"))?;

    let mut downloaded = resume_from;
    let emit = |downloaded: u64, total: u64| {
        let percentage = if total > 0 {
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        let _ = app.emit(
            "llm-download-progress",
            &DownloadProgress {
                model_id: id.to_string(),
                downloaded,
                total,
                percentage,
            },
        );
    };
    emit(downloaded, total);

    let mut last_emit = Instant::now();
    let throttle = Duration::from_millis(100);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download stream error: {e}"))?;
        file.write_all(&chunk)
            .map_err(|e| format!("Failed to write file: {e}"))?;
        downloaded += chunk.len() as u64;
        if last_emit.elapsed() >= throttle {
            emit(downloaded, total);
            last_emit = Instant::now();
        }
    }
    file.flush().map_err(|e| format!("Failed to flush: {e}"))?;
    drop(file);
    emit(downloaded.max(total), downloaded.max(total));

    // Verify size on completion.
    let actual = partial_path
        .metadata()
        .map_err(|e| format!("Failed to stat downloaded file: {e}"))?
        .len();
    if info.size_bytes > 0 && actual != info.size_bytes {
        let _ = fs::remove_file(&partial_path);
        return Err(format!(
            "Download incomplete: expected {} bytes, got {}",
            info.size_bytes, actual
        ));
    }

    fs::rename(&partial_path, &final_path)
        .map_err(|e| format!("Failed to finalize download: {e}"))?;
    info!("LLM download complete: {id}");
    Ok(())
}

/// Select a downloaded catalog model as the active local LLM by pointing `local_llm_model_path` at its
/// file. Rejected if the model isn't downloaded. This is the picker's whole integration with the AI
/// path — `ai_transform` reads that setting unchanged.
#[tauri::command]
#[specta::specta]
pub fn select_llm_model(app: AppHandle, id: String) -> Result<(), String> {
    let info = llm_catalog::find(&id).ok_or_else(|| format!("Unknown LLM model: {id}"))?;
    let path = llm_catalog::model_path(&app, &info);
    if !llm_catalog::file_is_complete(&path, info.size_bytes) {
        return Err(format!("Model not downloaded: {id}"));
    }

    let mut settings = get_settings(&app);
    settings.local_llm_model_path = path.to_string_lossy().to_string();
    write_settings(&app, settings);
    let _ = app.emit("llm-models-updated", ());
    Ok(())
}

/// Delete a downloaded catalog model's file. If it was the active model, also clears
/// `local_llm_model_path` so the AI chips fall back to disabled (or a cloud backend).
#[tauri::command]
#[specta::specta]
pub fn delete_llm_model(app: AppHandle, id: String) -> Result<(), String> {
    let info = llm_catalog::find(&id).ok_or_else(|| format!("Unknown LLM model: {id}"))?;
    let path = llm_catalog::model_path(&app, &info);

    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("Failed to delete model file: {e}"))?;
    }
    // Also drop any leftover partial.
    let partial = llm_catalog::llm_dir(&app).join(format!("{}.partial", info.filename));
    if partial.exists() {
        let _ = fs::remove_file(&partial);
    }

    // Clear the active selection if it pointed at this file.
    let mut settings = get_settings(&app);
    if std::path::PathBuf::from(settings.local_llm_model_path.trim()) == path {
        settings.local_llm_model_path = String::new();
        write_settings(&app, settings);
    }
    let _ = app.emit("llm-models-updated", ());
    Ok(())
}
