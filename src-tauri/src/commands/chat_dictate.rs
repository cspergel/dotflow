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
use crate::managers::transcription::TranscriptionManager;
use log::debug;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// Recording binding id used only for chat dictation. Kept distinct from the dictation hotkey's
/// `"transcribe"` binding so the two never match each other's start/stop in the audio manager's state.
const BINDING_ID: &str = "chat_dictate";

/// Begin recording the microphone for chat dictation. Kicks off a background model load (so the engine is
/// ready by the time [`chat_dictate_stop`] runs) and starts capture with offline VAD — the same
/// non-streaming path the batch dictation flow uses. Returns an error only if recording can't start (e.g.
/// microphone permission denied, or a dictation is already in progress).
#[tauri::command]
#[specta::specta]
pub fn chat_dictate_start(app: AppHandle) -> Result<(), String> {
    let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
    let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());

    // Load the ASR model in the background so it's warm by the time we stop.
    tm.initiate_model_load();

    // Offline VAD = the non-streaming batch path (no live stream worker). We transcribe the captured
    // samples in one shot on stop.
    rm.try_start_recording(BINDING_ID, VadPolicy::Offline)?;
    debug!("chat_dictate_start: recording started");
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
            return Ok(String::new());
        };
        if samples.is_empty() {
            debug!("chat_dictate_stop: recording produced no samples");
            return Ok(String::new());
        }
        tm.transcribe(samples).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("chat dictation task failed: {e}"))??;

    Ok(text.trim().to_string())
}
