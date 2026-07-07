//! DotFlow — Tauri command for the EXPERIMENTAL typed text expander toggle.
//!
//! Persists the `experimental_typed_expander` setting and starts/stops the global keyboard monitor to match,
//! so flipping the switch takes effect immediately (no restart). Off by default; the monitor watches typing,
//! so it is strictly opt-in.

use std::sync::Arc;
use tauri::{AppHandle, Manager};

use crate::dotflow::typed_expander::ExpanderController;
use crate::settings::{get_settings, write_settings};

#[tauri::command]
#[specta::specta]
pub fn change_typed_expander_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.experimental_typed_expander = enabled;
    write_settings(&app, settings);

    let controller = app
        .try_state::<Arc<ExpanderController>>()
        .ok_or("Typed-expander controller not initialized")?;
    if enabled {
        controller.start(app.clone());
    } else {
        controller.stop();
    }
    Ok(())
}
