//! Tauri global-shortcut implementation
//!
//! This module provides shortcut functionality using Tauri's built-in
//! global-shortcut plugin.

use log::{error, warn};
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[cfg(not(target_os = "linux"))]
use crate::settings::get_settings;
use crate::settings::{self, ShortcutBinding};

use super::handler::handle_shortcut_event;

/// Initialize shortcuts using Tauri's global-shortcut plugin
pub fn init_shortcuts(app: &AppHandle) {
    let default_bindings = settings::get_default_settings().bindings;
    let user_settings = settings::load_or_create_app_settings(app);

    // Register all default shortcuts, applying user customizations
    for (id, default_binding) in default_bindings {
        if id == "cancel" {
            continue; // Skip cancel shortcut, it will be registered dynamically
        }
        // Skip post-processing shortcut when the feature is disabled
        if id == "transcribe_with_post_process" && !user_settings.post_process_enabled {
            continue;
        }
        // Skip the selection-review shortcut when the feature is disabled
        if id == "review_selection" && !user_settings.selection_review_enabled {
            continue;
        }
        let binding = user_settings
            .bindings
            .get(&id)
            .cloned()
            .unwrap_or(default_binding);

        if let Err(e) = register_shortcut(app, binding) {
            error!("Failed to register shortcut {} during init: {}", id, e);
        }
    }
}

/// Validate a shortcut string for the Tauri global-shortcut implementation.
/// Tauri requires at least one non-modifier key and doesn't support the fn key.
pub fn validate_shortcut(raw: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Err("Shortcut cannot be empty".into());
    }

    let modifiers = [
        "ctrl", "control", "shift", "alt", "option", "meta", "command", "cmd", "super", "win",
        "windows",
    ];

    // Check for fn key which Tauri doesn't support
    let parts: Vec<String> = raw.split('+').map(|p| p.trim().to_lowercase()).collect();
    for part in &parts {
        if part == "fn" || part == "function" {
            return Err("The 'fn' key is not supported by Tauri global shortcuts".into());
        }
    }

    // Check for at least one non-modifier key
    let has_non_modifier = parts.iter().any(|part| !modifiers.contains(&part.as_str()));
    if !has_non_modifier {
        return Err("Shortcuts must include a main key (letter, number, F-key, etc.) in addition to modifiers".into());
    }

    // DotFlow: a persistent GLOBAL shortcut must include a modifier. A bare key like "l" would fire on every
    // press of that key system-wide — it makes the app unusable and traps the user (typing re-triggers it).
    // Bare Escape and F-keys are exempt: they aren't typed into text, so they carry no such footgun (and the
    // `cancel` binding legitimately uses a bare "escape").
    let has_modifier = parts.iter().any(|part| modifiers.contains(&part.as_str()));
    if !has_modifier && !is_exempt_bare_key(&parts) {
        return Err(
            "Shortcut must include a modifier (Ctrl, Alt, Shift, or Win) — a bare key would fire on every keypress"
                .into(),
        );
    }

    Ok(())
}

/// Keys allowed as a global shortcut with NO modifier: Escape, and F1–F24. Everything else typable needs a
/// modifier (see `validate_shortcut`).
fn is_exempt_bare_key(parts: &[String]) -> bool {
    if parts.len() != 1 {
        return false;
    }
    let key = parts[0].as_str();
    if key == "escape" || key == "esc" {
        return true;
    }
    // F1–F24
    key.strip_prefix('f')
        .and_then(|n| n.parse::<u32>().ok())
        .is_some_and(|n| (1..=24).contains(&n))
}

/// Register a shortcut using Tauri's global-shortcut plugin
pub fn register_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    // Validate for Tauri requirements
    if let Err(e) = validate_shortcut(&binding.current_binding) {
        warn!(
            "register_tauri_shortcut validation error for binding '{}': {}",
            binding.current_binding, e
        );
        return Err(e);
    }

    // Parse shortcut and return error if it fails
    let shortcut = match binding.current_binding.parse::<Shortcut>() {
        Ok(s) => s,
        Err(e) => {
            let error_msg = format!(
                "Failed to parse shortcut '{}': {}",
                binding.current_binding, e
            );
            error!("register_tauri_shortcut parse error: {}", error_msg);
            return Err(error_msg);
        }
    };

    // Prevent duplicate registrations that would silently shadow one another
    if app.global_shortcut().is_registered(shortcut) {
        let error_msg = format!("Shortcut '{}' is already in use", binding.current_binding);
        warn!("register_tauri_shortcut duplicate error: {}", error_msg);
        return Err(error_msg);
    }

    // Clone binding.id for use in the closure
    let binding_id_for_closure = binding.id.clone();

    app.global_shortcut()
        .on_shortcut(shortcut, move |app_handle, scut, event| {
            if scut == &shortcut {
                let shortcut_string = scut.into_string();
                let is_pressed = event.state == ShortcutState::Pressed;
                handle_shortcut_event(
                    app_handle,
                    &binding_id_for_closure,
                    &shortcut_string,
                    is_pressed,
                );
            }
        })
        .map_err(|e| {
            let error_msg = format!(
                "Couldn't register shortcut '{}': {}",
                binding.current_binding, e
            );
            error!("register_tauri_shortcut registration error: {}", error_msg);
            error_msg
        })?;

    Ok(())
}

/// Unregister a shortcut from Tauri's global-shortcut plugin
pub fn unregister_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    let shortcut = match binding.current_binding.parse::<Shortcut>() {
        Ok(s) => s,
        Err(e) => {
            let error_msg = format!(
                "Failed to parse shortcut '{}' for unregistration: {}",
                binding.current_binding, e
            );
            error!("unregister_tauri_shortcut parse error: {}", error_msg);
            return Err(error_msg);
        }
    };

    app.global_shortcut().unregister(shortcut).map_err(|e| {
        let error_msg = format!(
            "Failed to unregister shortcut '{}': {}",
            binding.current_binding, e
        );
        error!("unregister_tauri_shortcut error: {}", error_msg);
        error_msg
    })?;

    Ok(())
}

/// Register the cancel shortcut (called when recording starts)
pub fn register_cancel_shortcut(app: &AppHandle) {
    // Cancel shortcut is disabled on Linux due to instability with dynamic shortcut registration
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        return;
    }

    #[cfg(not(target_os = "linux"))]
    {
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(cancel_binding) = get_settings(&app_clone).bindings.get("cancel").cloned() {
                if let Err(e) = register_shortcut(&app_clone, cancel_binding) {
                    error!("Failed to register cancel shortcut: {}", e);
                }
            }
        });
    }
}

/// Unregister the cancel shortcut (called when recording stops)
pub fn unregister_cancel_shortcut(app: &AppHandle) {
    // Cancel shortcut is disabled on Linux due to instability with dynamic shortcut registration
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        return;
    }

    #[cfg(not(target_os = "linux"))]
    {
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(cancel_binding) = get_settings(&app_clone).bindings.get("cancel").cloned() {
                // We ignore errors here as it might already be unregistered
                let _ = unregister_shortcut(&app_clone, cancel_binding);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::validate_shortcut;

    #[test]
    fn rejects_a_bare_letter() {
        // The exact footgun: "l" alone would fire on every L keypress system-wide.
        assert!(validate_shortcut("l").is_err());
        assert!(validate_shortcut("a").is_err());
        assert!(validate_shortcut("5").is_err());
        assert!(validate_shortcut("space").is_err());
    }

    #[test]
    fn accepts_modifier_combos() {
        assert!(validate_shortcut("ctrl+shift+u").is_ok());
        assert!(validate_shortcut("ctrl+space").is_ok());
        assert!(validate_shortcut("alt+shift+k").is_ok());
    }

    #[test]
    fn allows_bare_escape_and_fkeys() {
        // Not typed into text, so no footgun; `cancel` uses a bare "escape".
        assert!(validate_shortcut("escape").is_ok());
        assert!(validate_shortcut("f5").is_ok());
        assert!(validate_shortcut("f24").is_ok());
    }

    #[test]
    fn rejects_modifier_only_and_empty() {
        assert!(validate_shortcut("ctrl").is_err()); // no main key
        assert!(validate_shortcut("ctrl+shift").is_err());
        assert!(validate_shortcut("").is_err());
    }

    #[test]
    fn rejects_the_fn_key() {
        assert!(validate_shortcut("fn").is_err());
    }
}
