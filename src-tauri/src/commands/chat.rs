//! DotFlow — offline AI chat commands.
//!
//! Streams a local-LLM reply to the frontend **token-by-token** via events so the chat panel renders as the
//! model generates. The heavy inference runs on a blocking thread (the async runtime is never stalled); the
//! model cache in [`crate::dotflow::local_llm`] serializes concurrent generations. Feature-gated on
//! `local-llm` — in a build without it, `chat_available` is `false` and `chat_stream` returns a clear error.
//!
//! Events emitted (frontend listens on these):
//! - `chat-token` `{ id, text }` — one decoded piece; append to the live assistant message.
//! - `chat-done`  `{ id, text }` — generation finished; `text` is the full **cleaned** reply (authoritative,
//!   replaces the streamed text so any stray template marker is gone).
//! - `chat-error` `{ id, message }` — generation failed or produced nothing.
//!
//! `id` correlates a turn: the frontend passes a fresh id per send so a stale/cancelled stream's late events
//! can be ignored, and `chat_cancel(id)` cooperatively stops that turn.

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashSet;
use std::sync::Mutex;
use tauri::AppHandle;

/// One message in the conversation, as sent from the frontend. `role` is `"system" | "user" | "assistant"`.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Payload for `chat-token`.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ChatTokenEvent {
    pub id: u64,
    pub text: String,
}

/// Payload for `chat-done`.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ChatDoneEvent {
    pub id: u64,
    pub text: String,
}

/// Payload for `chat-error`.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ChatErrorEvent {
    pub id: u64,
    pub message: String,
}

/// Turn ids the user asked to cancel. `chat_cancel` inserts; the generation loop polls `contains`; the
/// `chat_stream` task removes the id when it finishes. A `Mutex<HashSet>` is plenty — cancels are rare.
static CHAT_CANCEL: Lazy<Mutex<HashSet<u64>>> = Lazy::new(|| Mutex::new(HashSet::new()));

fn cancel_requested(id: u64) -> bool {
    CHAT_CANCEL
        .lock()
        .map(|s| s.contains(&id))
        .unwrap_or(false)
}

fn clear_cancel(id: u64) {
    if let Ok(mut s) = CHAT_CANCEL.lock() {
        s.remove(&id);
    }
}

/// Request that the in-flight chat turn `id` stop generating. The generation loop polls this and ends early;
/// a `chat-done` with whatever was produced so far still fires.
#[tauri::command]
#[specta::specta]
pub fn chat_cancel(id: u64) -> Result<(), String> {
    CHAT_CANCEL
        .lock()
        .map_err(|_| "cancel registry poisoned".to_string())?
        .insert(id);
    Ok(())
}

/// Whether offline chat is usable: only in `local-llm` builds, and only when a local model is selected and
/// its file exists on disk.
#[tauri::command]
#[specta::specta]
pub fn chat_available(app: AppHandle) -> bool {
    #[cfg(feature = "local-llm")]
    {
        let settings = crate::settings::get_settings(&app);
        let path = settings.local_llm_model_path.trim();
        return !path.is_empty() && std::path::Path::new(path).exists();
    }
    #[cfg(not(feature = "local-llm"))]
    {
        let _ = app;
        false
    }
}

/// Stream a chat reply for the conversation `messages`, using the currently-selected local model. Emits
/// `chat-token` per piece, then `chat-done` (or `chat-error`). Returns immediately-ish: the await is only the
/// blocking generation task completing. `id` is the frontend-chosen turn id (also used by `chat_cancel`).
#[tauri::command]
#[specta::specta]
pub async fn chat_stream(
    app: AppHandle,
    id: u64,
    messages: Vec<ChatMessage>,
) -> Result<(), String> {
    #[cfg(not(feature = "local-llm"))]
    {
        let _ = (&app, id, &messages);
        Err("This build was compiled without local model support.".to_string())
    }

    #[cfg(feature = "local-llm")]
    {
        use crate::dotflow::local_llm::{self, ChatTurn, Role};
        use tauri::Emitter;

        if messages.is_empty() {
            return Err("No messages to send.".to_string());
        }

        let settings = crate::settings::get_settings(&app);
        let path = settings.local_llm_model_path.trim().to_string();
        if path.is_empty() {
            return Err("No local model selected — pick one in the model dropdown.".to_string());
        }
        let model_path = std::path::PathBuf::from(&path);
        if !model_path.exists() {
            return Err(format!("Model file not found: {path}"));
        }

        let turns: Vec<ChatTurn> = messages
            .iter()
            .map(|m| ChatTurn {
                role: match m.role.as_str() {
                    "system" => Role::System,
                    "assistant" => Role::Assistant,
                    _ => Role::User,
                },
                content: m.content.clone(),
            })
            .collect();

        // Drop any stale cancel flag from a previous turn that reused this id.
        clear_cancel(id);

        let app_task = app.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let app_tok = app_task.clone();
            local_llm::generate_chat_stream(
                &model_path,
                &turns,
                1024,
                |piece| {
                    let _ = app_tok.emit(
                        "chat-token",
                        ChatTokenEvent {
                            id,
                            text: piece.to_string(),
                        },
                    );
                },
                &|| cancel_requested(id),
            )
        })
        .await
        .map_err(|e| format!("chat generation task failed: {e}"))?;

        clear_cancel(id);

        match result {
            Ok(text) => {
                let _ = app.emit("chat-done", ChatDoneEvent { id, text });
                Ok(())
            }
            Err(e) => {
                let _ = app.emit(
                    "chat-error",
                    ChatErrorEvent {
                        id,
                        message: e.clone(),
                    },
                );
                Err(e)
            }
        }
    }
}
