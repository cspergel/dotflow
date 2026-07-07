//! DotFlow — Tauri commands for the editable phrase library (the in-app "custom dictation inserts" UI,
//! Beeftext-simple: trigger → text block). All CRUD goes through `PhraseManager`, which rebuilds the
//! compiled table the live dictation reads, so an edit here takes effect on the next spoken trigger.

use crate::managers::phrases::{PhraseManager, PhraseRecord};
use std::sync::Arc;
use tauri::State;

#[tauri::command]
#[specta::specta]
pub async fn get_phrases(
    phrase_manager: State<'_, Arc<PhraseManager>>,
) -> Result<Vec<PhraseRecord>, String> {
    phrase_manager.list().map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn add_phrase(
    phrase_manager: State<'_, Arc<PhraseManager>>,
    key: String,
    aliases: Vec<String>,
    expansion: String,
) -> Result<PhraseRecord, String> {
    phrase_manager
        .add(key, aliases, expansion)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn update_phrase(
    phrase_manager: State<'_, Arc<PhraseManager>>,
    id: i64,
    key: String,
    aliases: Vec<String>,
    expansion: String,
) -> Result<PhraseRecord, String> {
    phrase_manager
        .update(id, key, aliases, expansion)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_phrase(
    phrase_manager: State<'_, Arc<PhraseManager>>,
    id: i64,
) -> Result<(), String> {
    phrase_manager.delete(id).map_err(|e| e.to_string())
}
