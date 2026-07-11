//! DotFlow — LLM router. Dispatches every local-LLM call to the **sidecar** (crash-isolated, 32k) when it's
//! healthy, else the **in-process** model (16k). Checking health per call means a sidecar that dies mid-session
//! falls back cleanly on the next request. With no sidecar present this is byte-for-byte the old in-process
//! behavior — the sidecar is purely additive.

use tauri::{AppHandle, Manager};

use crate::dotflow::sidecar::{self, SidecarManager};

/// The sidecar's base URL when it's up and healthy, else `None` (→ use the in-process path).
pub fn sidecar_base_url(app: &AppHandle) -> Option<String> {
    app.try_state::<SidecarManager>().and_then(|m| m.base_url())
}

/// Single-turn generation for transforms / summarize (extraction + synthesis) — no visible reasoning wanted.
/// Routes to the sidecar (`enable_thinking:false` → direct, reliable answer, verified in Phase 0) when healthy,
/// else the in-process model at `model_path`. Sync because the callers run inside `spawn_blocking`; the async
/// sidecar call is bridged with `block_on` (safe here — a blocking-pool thread is not an async context).
pub fn generate_chat(
    app: &AppHandle,
    model_path: &std::path::Path,
    system: &str,
    user: &str,
    max_new: usize,
) -> Result<String, String> {
    if let Some(url) = sidecar_base_url(app) {
        let msgs = vec![
            ("system".to_string(), system.to_string()),
            ("user".to_string(), user.to_string()),
        ];
        // Bridge to the async HTTP call with a FRESH current-thread runtime. Callers run on the blocking pool
        // (via `spawn_blocking`), which has no ambient runtime — so a private runtime here `block_on`s safely,
        // avoiding the "cannot start a runtime from within a runtime" panic that a shared handle can hit.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to start async bridge: {e}"))?;
        return rt.block_on(sidecar::complete(
            &url,
            msgs,
            max_new as u32,
            false, // enable_thinking = false → direct answer
            false, // don't wrap reasoning (there is none)
        ));
    }

    #[cfg(feature = "local-llm")]
    {
        crate::dotflow::local_llm::generate_chat(model_path, system, user, max_new)
    }
    #[cfg(not(feature = "local-llm"))]
    {
        let _ = (model_path, system, user, max_new);
        Err("No local LLM backend available.".to_string())
    }
}
