//! DotFlow — chat mic dictation commands.
//!
//! Lets the chat UI record the microphone and get the transcript back as **text** instead of the normal
//! dictation flow which pastes into the foreground app. Reuses the exact same STT machinery the dictation
//! hotkey uses ([`AudioRecordingManager`] for capture, [`TranscriptionManager::transcribe`] for the
//! batch transcription) — the only difference is the result is RETURNED to the caller rather than injected.
//!
//! Feature-independent: these do NOT touch the `local-llm` feature; they only need the ASR pipeline.
//!
//! Flow: the frontend mic button calls [`chat_dictate_start`] on press and [`chat_dictate_stop`] on
//! release (or as a toggle), then drops the returned string into the chat input.

use crate::audio_toolkit::VadPolicy;
use crate::managers::audio::AudioRecordingManager;
use crate::managers::model::ModelManager;
use crate::managers::transcription::TranscriptionManager;
use log::debug;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// Recording binding id used only for chat dictation. Kept distinct from the dictation hotkey's
/// `"transcribe"` binding so the two never match each other's start/stop in the audio manager's state.
const BINDING_ID: &str = "chat_dictate";

/// Begin recording the microphone for chat dictation. Kicks off a background model load (so the engine is
/// ready by the time [`chat_dictate_stop`] runs). When the selected STT model supports streaming, we run the
/// live streaming path so partial text appears in the chat box as you speak (via the `stream-text` event,
/// with focused-field injection suppressed by chat-stream mode); otherwise we fall back to the one-shot batch
/// path. No overlay is shown either way. Returns an error only if recording can't start (e.g. microphone
/// permission denied, or a dictation is already in progress).
#[tauri::command]
#[specta::specta]
pub fn chat_dictate_start(app: AppHandle) -> Result<(), String> {
    let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
    let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());

    // Load the ASR model in the background so it's warm by the time we stop.
    tm.initiate_model_load();

    // Stream live into the chat box when the model supports it; otherwise use the batch path.
    let settings = crate::settings::get_settings(&app);
    let supports_streaming = app
        .state::<Arc<ModelManager>>()
        .get_model_info(&settings.selected_model)
        .map(|m| m.supports_streaming)
        .unwrap_or(false);

    let vad_policy = if supports_streaming {
        tm.set_chat_stream_mode(true); // suppress field injection; preview goes to the chat box only
        tm.start_stream();
        VadPolicy::Streaming
    } else {
        VadPolicy::Offline
    };

    if let Err(e) = rm.try_start_recording(BINDING_ID, vad_policy) {
        // Roll back any stream we started so the next start isn't blocked.
        tm.cancel_stream();
        tm.set_chat_stream_mode(false);
        return Err(e);
    }
    debug!("chat_dictate_start: recording started (streaming={supports_streaming})");
    Ok(())
}

/// Stop chat-dictation recording and return the transcript as text (trimmed). Runs the SAME batch
/// transcription pipeline the dictation hotkey uses, but returns the string instead of pasting it.
///
/// Idempotent-ish: if no chat dictation is active (never started, or already stopped) the recorder returns
/// no samples and this yields `Ok("")`. An empty capture or empty transcription is likewise `Ok("")`, not an
/// error — only a genuine transcription failure surfaces as `Err`.
#[tauri::command]
#[specta::specta]
pub async fn chat_dictate_stop(app: AppHandle) -> Result<String, String> {
    let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
    let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());

    // Snapshot the cancel generation so `stop_recording` can detect a concurrent cancel.
    let cancel_generation = rm.cancel_generation();

    // stop_recording (may sleep for the trailing-audio buffer) and transcribe are both blocking/CPU-bound,
    // so run them off the async runtime.
    let text = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let Some(samples) = rm.stop_recording(BINDING_ID, cancel_generation) else {
            debug!("chat_dictate_stop: no active chat recording / no samples");
            tm.cancel_stream(); // tear down any stream worker so it doesn't leak
            tm.set_chat_stream_mode(false);
            return Ok(String::new());
        };
        if samples.is_empty() {
            debug!("chat_dictate_stop: recording produced no samples");
            tm.cancel_stream();
            tm.set_chat_stream_mode(false);
            return Ok(String::new());
        }
        // If a live stream ran, finalize it and use its text (all audio was already fed to the stream);
        // otherwise batch-transcribe the samples. Mirrors the dictation-hotkey path in actions.rs.
        let result = match tm.finalize_stream() {
            Ok(Some(t)) if !t.trim().is_empty() => Ok(t),
            Ok(_) => tm.transcribe(samples).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        tm.set_chat_stream_mode(false);
        result
    })
    .await
    .map_err(|e| format!("chat dictation task failed: {e}"))??;

    Ok(text.trim().to_string())
}
