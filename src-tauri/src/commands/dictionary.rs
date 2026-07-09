//! DotFlow — commands for the dictionary-packs feature (Settings → Cleanup → Dictionaries).
//!
//! Packs extend Harper's vocabulary so valid domain terms are not flagged as misspellings. The bundled
//! `medical` pack is always available; users can drop additional `*.txt` packs into the dictionaries dir.
//! These commands list/toggle/reload packs and open the folder. Toggling writes the setting and rebuilds
//! the process-wide merged dictionary live (no restart). See [`crate::dotflow::dictionary_packs`].

use std::path::PathBuf;

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::dotflow::dictionary_packs::{self, DictionaryPackInfo};
use crate::settings::{get_settings, write_settings};

/// The dictionaries dir: `<app_data>/dictionaries` (portable-aware). Falls back to a relative path only if
/// the app data dir can't be resolved (should not happen in practice).
pub fn dictionaries_dir(app: &AppHandle) -> PathBuf {
    crate::portable::app_data_dir(app)
        .map(|d| d.join("dictionaries"))
        .unwrap_or_else(|_| PathBuf::from("dictionaries"))
}

/// List every discovered pack (bundled `medical` first, then user `*.txt` files) with its label, term
/// count, and enabled state. Best-effort creates the dir so the "Open folder" hint has a real target.
#[tauri::command]
#[specta::specta]
pub fn get_dictionary_packs(app: AppHandle) -> Vec<DictionaryPackInfo> {
    let dir = dictionaries_dir(&app);
    let _ = std::fs::create_dir_all(&dir); // best-effort ([RS2-F7] degrade, never panic)
    let enabled = get_settings(&app).enabled_dictionary_packs;
    dictionary_packs::pack_infos(&dir, &enabled)
}

/// Enable/disable a pack by id: writes the setting, then rebuilds the merged dictionary live.
#[tauri::command]
#[specta::specta]
pub fn set_dictionary_pack_enabled(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.enabled_dictionary_packs.retain(|x| x != &id);
    if enabled {
        settings.enabled_dictionary_packs.push(id);
    }
    let ids = settings.enabled_dictionary_packs.clone();
    write_settings(&app, settings);

    let dir = dictionaries_dir(&app);
    dictionary_packs::set_enabled_packs(&dir, &ids);
    Ok(())
}

/// Re-scan the dictionaries dir and force a rebuild of the merged dictionary — picks up newly dropped or
/// edited `*.txt` packs ([RS2-F5]). Returns the refreshed pack list.
#[tauri::command]
#[specta::specta]
pub fn reload_dictionary_packs(app: AppHandle) -> Vec<DictionaryPackInfo> {
    let dir = dictionaries_dir(&app);
    let _ = std::fs::create_dir_all(&dir);
    let enabled = get_settings(&app).enabled_dictionary_packs;
    dictionary_packs::set_enabled_packs(&dir, &enabled); // always rebuilds
    dictionary_packs::pack_infos(&dir, &enabled)
}

/// The user's custom accepted words ("My Words" pack, `dictionaries/custom.txt`).
#[tauri::command]
#[specta::specta]
pub fn get_custom_dictionary_words(app: AppHandle) -> Vec<String> {
    dictionary_packs::read_custom_words(&dictionaries_dir(&app))
}

/// Add a word to the custom pack, auto-enable the pack, and rebuild the merged dictionary live. Returns the
/// updated word list (or a user-facing error for bad input).
#[tauri::command]
#[specta::specta]
pub fn add_custom_dictionary_word(app: AppHandle, word: String) -> Result<Vec<String>, String> {
    let dir = dictionaries_dir(&app);
    let words = dictionary_packs::add_custom_word(&dir, &word)?;
    ensure_custom_pack_enabled(&app);
    let ids = get_settings(&app).enabled_dictionary_packs;
    dictionary_packs::set_enabled_packs(&dir, &ids);
    Ok(words)
}

/// Remove a word from the custom pack and rebuild the merged dictionary live. Returns the updated list.
#[tauri::command]
#[specta::specta]
pub fn remove_custom_dictionary_word(
    app: AppHandle,
    word: String,
) -> Result<Vec<String>, String> {
    let dir = dictionaries_dir(&app);
    let words = dictionary_packs::remove_custom_word(&dir, &word)?;
    let ids = get_settings(&app).enabled_dictionary_packs;
    dictionary_packs::set_enabled_packs(&dir, &ids);
    Ok(words)
}

/// Ensure the `custom` pack is in the enabled set so newly added words take effect immediately.
fn ensure_custom_pack_enabled(app: &AppHandle) {
    let mut settings = get_settings(app);
    if !settings
        .enabled_dictionary_packs
        .iter()
        .any(|x| x == dictionary_packs::CUSTOM_PACK_ID)
    {
        settings
            .enabled_dictionary_packs
            .push(dictionary_packs::CUSTOM_PACK_ID.to_string());
        write_settings(app, settings);
    }
}

/// Reveal the dictionaries dir in the OS file manager so users can drop in their own `.txt` term lists.
#[tauri::command]
#[specta::specta]
pub fn open_dictionaries_folder(app: AppHandle) -> Result<(), String> {
    let dir = dictionaries_dir(&app);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create dictionaries folder: {e}"))?;
    let path = dir.to_string_lossy().as_ref().to_string();
    app.opener()
        .open_path(path, None::<String>)
        .map_err(|e| format!("Failed to open dictionaries folder: {e}"))?;
    Ok(())
}
