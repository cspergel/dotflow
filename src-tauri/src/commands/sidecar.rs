//! DotFlow — Tauri commands for the `llama-server` sidecar (Phase 1).
//!
//! `sidecar_ensure_started` is called when the user opens the chat (lazy spawn); it resolves the chat model
//! (+ an mmproj sibling for vision, if present) and asks the [`SidecarManager`] to spawn + health-check the
//! server. `sidecar_status` lets the frontend render the backend badge on mount. Both are cheap and safe to
//! call repeatedly — the manager is idempotent and degrades to the in-process engine on any problem.

use tauri::{AppHandle, Manager};

use crate::dotflow::sidecar::{BackendStatus, SidecarManager};
use crate::settings::get_settings;

/// Find an mmproj (vision projector) GGUF sitting next to the chat model, if the user has downloaded one.
/// Enables Qwythos vision without extra configuration; `None` (the common case for now) just means text-only.
fn find_mmproj(model_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let dir = model_path.parent()?;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            let lower = name.to_ascii_lowercase();
            if lower.starts_with("mmproj") && lower.ends_with(".gguf") {
                return Some(p);
            }
        }
    }
    None
}

/// Ensure the sidecar is started + healthy, returning the resulting backend status. Called when the chat becomes
/// active. Idempotent; never errors — a missing binary / model just leaves the in-process fallback active.
#[tauri::command]
#[specta::specta]
pub async fn sidecar_ensure_started(app: AppHandle) -> Result<BackendStatus, String> {
    let manager = app.state::<SidecarManager>().inner().clone();
    let model = get_settings(&app).local_llm_model_path.trim().to_string();
    if !model.is_empty() {
        let model_path = std::path::PathBuf::from(&model);
        let mmproj = find_mmproj(&model_path);
        manager.ensure_started(&app, model_path, mmproj).await;
    }
    Ok(manager.status())
}

/// Current backend status (in-process vs sidecar, and the active context window) for the header badge.
#[tauri::command]
#[specta::specta]
pub fn sidecar_status(app: AppHandle) -> BackendStatus {
    app.state::<SidecarManager>().status()
}
