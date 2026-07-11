//! DotFlow — `llama-server` sidecar manager (Phase 1 of the sidecar design).
//!
//! Runs llama.cpp's `llama-server` as a **crash-isolated subprocess** exposing an OpenAI-compatible HTTP API.
//! A CUDA fault (OOM / flash-attn abort) then fails one HTTP request instead of aborting the whole app — the
//! in-process `llama-cpp-2` path can't survive that. The sidecar also serves a **32k context** (the real fix
//! for big-chart completeness) and, later, vision.
//!
//! Phase 1 scope: **detect** the binary, **spawn** it (auto free port, GPU, 32k), **health-check** it, track and
//! **broadcast status** (`llm-backend-status` events drive the chat badge + a fallback toast), and **shut it
//! down** cleanly. Request routing is Phase 2. Everything degrades to the in-process path, so a missing/broken
//! sidecar never breaks the app.
//!
//! The binary is user-fetched (prebuilt llama.cpp win-cuda release) and lives in `llama-server/` next to the exe
//! — auto-detected like Tesseract, and isolated from the in-process ggml DLLs (avoids the ggml-base.dll clash).

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// The context window the sidecar is launched with (the whole point — far past the in-process 16k cap).
pub const SIDECAR_CTX: u32 = 32768;
/// The in-process fallback's context cap, reported to the UI when the sidecar isn't the active backend.
pub const INPROCESS_CTX: u32 = 16384;

/// Which backend is currently answering, surfaced to the frontend as the `llm-backend-status` event so the chat
/// header can show `⚡ 32k · GPU sidecar` vs `⚠ 16k · in-process (fallback)` and toast on a fallback.
#[derive(Clone, Debug, Serialize, PartialEq, specta::Type)]
pub struct BackendStatus {
    /// `"sidecar"` or `"in-process"`.
    pub backend: String,
    /// The active context window (32768 for the sidecar, 16384 in-process).
    pub ctx: u32,
    /// Human-readable explanation (e.g. "GPU sidecar ready", "sidecar binary not found — using in-process").
    pub reason: String,
    /// True while the sidecar is spawning + health-checking (badge can show a spinner).
    pub starting: bool,
}

impl BackendStatus {
    fn in_process(reason: impl Into<String>) -> Self {
        Self {
            backend: "in-process".into(),
            ctx: INPROCESS_CTX,
            reason: reason.into(),
            starting: false,
        }
    }
    fn starting() -> Self {
        Self {
            backend: "in-process".into(),
            ctx: INPROCESS_CTX,
            reason: "Starting the AI engine…".into(),
            starting: true,
        }
    }
    fn sidecar(port: u16) -> Self {
        Self {
            backend: "sidecar".into(),
            ctx: SIDECAR_CTX,
            reason: format!("GPU sidecar ready on port {port}"),
            starting: false,
        }
    }
}

/// Internal, lock-guarded state. The child handle is kept so we can kill it on shutdown / model change.
struct Inner {
    child: Option<Child>,
    port: Option<u16>,
    status: BackendStatus,
}

/// Process-wide manager for the `llama-server` sidecar. Cloneable (shared `Arc`), stored in Tauri state.
#[derive(Clone)]
pub struct SidecarManager {
    inner: Arc<Mutex<Inner>>,
}

impl Default for SidecarManager {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                child: None,
                port: None,
                status: BackendStatus::in_process("Sidecar not started — using in-process engine"),
            })),
        }
    }
}

/// Locate `llama-server.exe`: `DOTFLOW_LLAMA_SERVER` env → `llama-server/llama-server.exe` next to the app exe →
/// same folder as the exe. Returns `None` if not present (→ the app runs in-process, no error).
pub fn find_llama_server() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("DOTFLOW_LLAMA_SERVER") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    for rel in ["llama-server/llama-server.exe", "llama-server.exe"] {
        let p = exe_dir.join(rel);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Pick a free TCP port on loopback by binding to `:0` and reading the assigned port, then releasing it. A tiny
/// TOCTOU window exists before the child re-binds it, which is acceptable for a local single-user app.
fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("couldn't find a free port: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("couldn't read chosen port: {e}"))?
        .port();
    Ok(port)
}

/// On Windows, don't flash a console window for the child server.
#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}
#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

impl SidecarManager {
    /// Current backend status (cheap; clones the small struct).
    pub fn status(&self) -> BackendStatus {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status
            .clone()
    }

    /// The sidecar's base URL when healthy (`http://127.0.0.1:<port>`), else `None`. Phase 2's router uses this.
    pub fn base_url(&self) -> Option<String> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.status.backend == "sidecar" {
            g.port.map(|p| format!("http://127.0.0.1:{p}"))
        } else {
            None
        }
    }

    /// True when the sidecar is up and healthy (router should prefer it).
    pub fn is_healthy(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status
            .backend
            == "sidecar"
    }

    fn set_status(&self, app: &AppHandle, status: BackendStatus) {
        {
            let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if g.status == status {
                return; // no change → don't spam events
            }
            g.status = status.clone();
        }
        let _ = app.emit("llm-backend-status", status);
    }

    /// Ensure the sidecar is started and healthy. Idempotent: a no-op when already healthy or mid-start.
    /// Spawns `llama-server` (auto free port, the chat model, 32k, all GPU layers), then polls `/health` up to
    /// ~60s. On failure it leaves status on the in-process fallback (+ a reason) — never an error to the caller.
    /// `model_path` is the GGUF to serve (the chat model); `mmproj` is the optional vision projector.
    pub async fn ensure_started(
        &self,
        app: &AppHandle,
        model_path: PathBuf,
        mmproj: Option<PathBuf>,
    ) {
        // Fast-path: already healthy or already starting → nothing to do.
        {
            let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if g.status.backend == "sidecar" || g.status.starting {
                return;
            }
        }

        let Some(bin) = find_llama_server() else {
            self.set_status(
                app,
                BackendStatus::in_process(
                    "Sidecar binary not found (llama-server/) — using in-process engine",
                ),
            );
            return;
        };
        if !model_path.exists() {
            self.set_status(
                app,
                BackendStatus::in_process(
                    "No model selected for the sidecar — using in-process engine",
                ),
            );
            return;
        }

        self.set_status(app, BackendStatus::starting());

        let port = match free_port() {
            Ok(p) => p,
            Err(e) => {
                self.set_status(
                    app,
                    BackendStatus::in_process(format!("{e} — using in-process engine")),
                );
                return;
            }
        };

        // Spawn. `--jinja` enables the model's chat template (so `enable_thinking` works); q8 KV + flash-attn
        // keep 32k inside 16GB. The binary's own ggml-cuda DLLs sit beside it; the child inherits our PATH
        // (which the launcher points at the CUDA toolkit) so it finds cudart.
        let mut cmd = Command::new(&bin);
        cmd.arg("-m")
            .arg(&model_path)
            .args(["--host", "127.0.0.1"])
            .args(["--port", &port.to_string()])
            .args(["-c", &SIDECAR_CTX.to_string()])
            .args(["-ngl", "999"])
            .args(["--flash-attn", "on"])
            .args(["--cache-type-k", "q8_0"])
            .args(["--cache-type-v", "q8_0"])
            .arg("--jinja");
        if let Some(mm) = mmproj.filter(|p| p.exists()) {
            cmd.arg("--mmproj").arg(mm);
        }
        if let Some(dir) = bin.parent() {
            cmd.current_dir(dir); // so the server finds its sibling ggml-cuda DLLs
        }
        no_window(&mut cmd);

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.set_status(
                    app,
                    BackendStatus::in_process(format!(
                        "Sidecar failed to start: {e} — using in-process engine"
                    )),
                );
                return;
            }
        };
        {
            let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            g.child = Some(child);
            g.port = Some(port);
        }

        // Health-poll /health until ready (~60s ceiling: model load can be slow).
        let url = format!("http://127.0.0.1:{port}/health");
        let client = reqwest::Client::new();
        let mut healthy = false;
        for _ in 0..120 {
            if let Ok(resp) = client
                .get(&url)
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await
            {
                if resp.status().is_success() {
                    healthy = true;
                    break;
                }
            }
            // Bail early if the child already died.
            if self.child_exited() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        if healthy {
            self.set_status(app, BackendStatus::sidecar(port));
        } else {
            self.kill_child();
            self.set_status(
                app,
                BackendStatus::in_process(
                    "Sidecar didn't become ready — using in-process engine (16k). Long summaries may be less complete.",
                ),
            );
        }
    }

    /// Whether the spawned child has already exited (so health-polling can stop early).
    fn child_exited(&self) -> bool {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match g.child.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(Some(_))),
            None => true,
        }
    }

    /// Kill the child process (best-effort) and clear the port. Called on failure, shutdown, or model change.
    pub fn kill_child(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut c) = g.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        g.port = None;
    }

    /// Stop the sidecar and reset to the in-process status. Call on app exit / model change.
    pub fn shutdown(&self) {
        self.kill_child();
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.status = BackendStatus::in_process("Sidecar stopped");
    }
}

/// Non-streaming chat completion against the sidecar's OpenAI-compatible endpoint.
///
/// `enable_thinking=false` makes Qwythos answer directly (verified in Phase 0 — the reliable path for
/// extraction / synthesis / transforms). When `wrap_reasoning` is true and the model DID think (thinking on),
/// the separated `reasoning_content` is re-wrapped as `<think>…</think>` before the answer so the chat UI's
/// existing think-parsing works unchanged. Returns the assistant text (possibly with a leading think block).
pub async fn complete(
    base_url: &str,
    messages: Vec<(String, String)>,
    max_tokens: u32,
    enable_thinking: bool,
    wrap_reasoning: bool,
) -> Result<String, String> {
    let msgs: Vec<serde_json::Value> = messages
        .into_iter()
        .map(|(role, content)| serde_json::json!({ "role": role, "content": content }))
        .collect();
    let body = serde_json::json!({
        "messages": msgs,
        "max_tokens": max_tokens,
        "stream": false,
        "chat_template_kwargs": { "enable_thinking": enable_thinking },
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base_url}/v1/chat/completions"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(600))
        .send()
        .await
        .map_err(|e| format!("sidecar request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("sidecar returned HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("sidecar response parse failed: {e}"))?;

    let msg = &v["choices"][0]["message"];
    let content = msg["content"].as_str().unwrap_or("");
    let reasoning = msg["reasoning_content"].as_str().unwrap_or("");
    if wrap_reasoning && !reasoning.trim().is_empty() {
        Ok(format!("<think>{reasoning}</think>{content}"))
    } else {
        Ok(content.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh manager reports the in-process backend at 16k (never claims the sidecar before it's up).
    #[test]
    fn default_status_is_in_process_16k() {
        let m = SidecarManager::default();
        let s = m.status();
        assert_eq!(s.backend, "in-process");
        assert_eq!(s.ctx, INPROCESS_CTX);
        assert!(!m.is_healthy());
        assert!(m.base_url().is_none(), "no base URL until healthy");
    }

    /// free_port must return a genuinely bindable loopback port (the smoke test showed a hardcoded port can be
    /// taken). Binding it again must succeed once the probe releases it.
    #[test]
    fn free_port_is_actually_free() {
        let p = free_port().expect("should find a free port");
        assert!(p >= 1024, "should be a non-privileged port, got {p}");
        // The probe released it, so we can bind it now.
        std::net::TcpListener::bind(("127.0.0.1", p)).expect("probed port should be bindable");
    }

    /// The status event shapes must carry the right ctx per backend (the badge relies on this).
    #[test]
    fn status_ctx_matches_backend() {
        assert_eq!(BackendStatus::sidecar(1234).ctx, SIDECAR_CTX);
        assert_eq!(BackendStatus::sidecar(1234).backend, "sidecar");
        assert_eq!(BackendStatus::in_process("x").ctx, INPROCESS_CTX);
        assert!(BackendStatus::starting().starting);
    }
}
