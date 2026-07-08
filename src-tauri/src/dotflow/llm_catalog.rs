//! DotFlow — the curated catalog of optional local LLMs that power the review overlay's AI actions
//! (Rewrite / Formal / Summarize). This is pure catalog + on-disk-status logic: it names a small,
//! hand-picked set of GGUF instruct models, tells the frontend which are downloaded and which is
//! active, and resolves where each lives on disk.
//!
//! This module is deliberately **feature-independent** — it does NOT depend on the `local-llm`
//! cargo feature. Downloading, selecting, and deleting a model are just file/settings management, so
//! the picker works in default builds too; only the actual inference (`dotflow::local_llm`) needs the
//! feature. Selecting a model simply points `settings.local_llm_model_path` at its downloaded file, so
//! `ai_transform` picks it up unchanged.
//!
//! Models live under `%APPDATA%/com.dotflow.app/models/llm/<filename>` (portable-aware via
//! [`crate::portable::app_data_dir`]). A model counts as "downloaded" when that file exists with the
//! catalog's exact `size_bytes` (see [`file_is_complete`]).

use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

/// One curated local LLM the user can download to power the AI-transform actions. The first eleven
/// fields are the static catalog spec; `downloaded` and `active` are live status filled in per request
/// (see [`crate::commands::llm::list_llm_models`]).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct LlmModelInfo {
    /// Stable catalog id (also the frontend key). Not the filename.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Parameter count, for display (e.g. "1.5B").
    pub params: String,
    /// Exact size of the GGUF in bytes — used both for the size badge and to verify a completed download.
    pub size_bytes: u64,
    /// SPDX-ish license label shown as a badge (e.g. "Apache-2.0", "MIT", "Qwen Research License").
    pub license: String,
    /// Whether the license permits commercial use. `false` models are badged non-commercial.
    pub commercial_ok: bool,
    /// Whether this is the recommended default (highlighted in the UI). Exactly one entry is `true`.
    pub recommended: bool,
    /// Short editorial note (one line) describing the trade-off.
    pub note: String,
    /// Direct download URL (bartowski GGUF, Q4_K_M quant). Verified to resolve to HTTP 200.
    pub url: String,
    /// On-disk filename under the llm dir.
    pub filename: String,
    /// Live: the file exists in the llm dir with the exact `size_bytes`.
    pub downloaded: bool,
    /// Live: this model's on-disk path equals `settings.local_llm_model_path`.
    pub active: bool,
}

/// The curated catalog. Every URL was verified (`curl -sIL` → HTTP 200 + matching `content-length`)
/// before inclusion. All are bartowski Q4_K_M GGUFs. `downloaded`/`active` start `false` and are filled
/// in by the listing command against the on-disk state.
pub fn catalog() -> Vec<LlmModelInfo> {
    vec![
        LlmModelInfo {
            id: "qwen2.5-1.5b-instruct".to_string(),
            name: "Qwen2.5 1.5B Instruct".to_string(),
            params: "1.5B".to_string(),
            size_bytes: 986_048_768,
            license: "Apache-2.0".to_string(),
            commercial_ok: true,
            recommended: true,
            note: "Clean (Apache-2.0), fast, and a small download. The reliable default — works with the current offline engine."
                .to_string(),
            url: "https://huggingface.co/bartowski/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/Qwen2.5-1.5B-Instruct-Q4_K_M.gguf"
                .to_string(),
            filename: "Qwen2.5-1.5B-Instruct-Q4_K_M.gguf".to_string(),
            downloaded: false,
            active: false,
        },
        LlmModelInfo {
            id: "qwen2.5-3b-instruct".to_string(),
            name: "Qwen2.5 3B Instruct".to_string(),
            params: "3B".to_string(),
            size_bytes: 1_929_903_264,
            license: "Qwen Research License".to_string(),
            commercial_ok: false,
            recommended: false,
            note: "Best quality at this size, but NON-COMMERCIAL (Qwen Research License) — personal use only."
                .to_string(),
            url: "https://huggingface.co/bartowski/Qwen2.5-3B-Instruct-GGUF/resolve/main/Qwen2.5-3B-Instruct-Q4_K_M.gguf"
                .to_string(),
            filename: "Qwen2.5-3B-Instruct-Q4_K_M.gguf".to_string(),
            downloaded: false,
            active: false,
        },
        LlmModelInfo {
            id: "phi-3.5-mini-instruct".to_string(),
            name: "Phi-3.5 Mini Instruct".to_string(),
            params: "3.8B".to_string(),
            size_bytes: 2_393_232_672,
            license: "MIT".to_string(),
            commercial_ok: true,
            recommended: false,
            note: "Capable and clean (MIT). Larger download; strong instruction-following.".to_string(),
            url: "https://huggingface.co/bartowski/Phi-3.5-mini-instruct-GGUF/resolve/main/Phi-3.5-mini-instruct-Q4_K_M.gguf"
                .to_string(),
            filename: "Phi-3.5-mini-instruct-Q4_K_M.gguf".to_string(),
            downloaded: false,
            active: false,
        },
    ]
}

/// Look up a catalog entry by id (base spec, without live status).
pub fn find(id: &str) -> Option<LlmModelInfo> {
    catalog().into_iter().find(|m| m.id == id)
}

/// The directory local LLMs live in: `<app_data>/models/llm` (portable-aware). Falls back to a
/// relative path only if the app data dir can't be resolved (should not happen in practice).
pub fn llm_dir(app: &AppHandle) -> PathBuf {
    crate::portable::app_data_dir(app)
        .map(|d| d.join("models").join("llm"))
        .unwrap_or_else(|_| PathBuf::from("models").join("llm"))
}

/// Absolute on-disk path where a catalog model's GGUF lives (whether or not it's present).
pub fn model_path(app: &AppHandle, info: &LlmModelInfo) -> PathBuf {
    llm_dir(app).join(&info.filename)
}

/// True when `path` exists and its length matches `expected_size` exactly — the "downloaded" test.
/// The exact-size check rejects a truncated / interrupted download that never got renamed off `.partial`
/// but somehow occupies the final path.
pub fn file_is_complete(path: &Path, expected_size: u64) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && (expected_size == 0 || meta.len() == expected_size),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_well_formed() {
        let cat = catalog();
        assert!(cat.len() >= 3, "expected at least 3 curated models");

        // Ids are unique.
        let mut ids: Vec<&str> = cat.iter().map(|m| m.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "catalog ids must be unique");

        // Exactly one recommended default, and it must be commercially usable (never default users onto a
        // non-commercial model) AND must load on the bundled engine. Currently Qwen2.5 1.5B (Apache-2.0);
        // Gemma 4 is excluded until llama-cpp-2's llama.cpp supports the `gemma4` architecture.
        let recommended: Vec<&LlmModelInfo> = cat.iter().filter(|m| m.recommended).collect();
        assert_eq!(recommended.len(), 1, "exactly one recommended default");
        assert_eq!(recommended[0].id, "qwen2.5-1.5b-instruct");
        assert!(
            recommended[0].commercial_ok,
            "the recommended default must be commercially licensed"
        );

        // The Qwen 3B entry is explicitly non-commercial.
        let qwen3b = cat.iter().find(|m| m.id == "qwen2.5-3b-instruct").unwrap();
        assert!(!qwen3b.commercial_ok);

        // Every entry has a plausible spec.
        for m in &cat {
            assert!(!m.name.is_empty());
            assert!(m.size_bytes > 0);
            assert!(m.url.starts_with("https://"));
            assert!(m.filename.ends_with(".gguf"));
            assert!(
                !m.downloaded && !m.active,
                "static catalog carries no live status"
            );
        }
    }

    #[test]
    fn find_resolves_known_and_unknown_ids() {
        assert!(find("phi-3.5-mini-instruct").is_some());
        assert!(find("does-not-exist").is_none());
    }

    #[test]
    fn file_is_complete_matches_exact_size_only() {
        let dir = std::env::temp_dir().join(format!("dotflow-llm-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("m.gguf");
        std::fs::write(&path, b"hello").unwrap(); // 5 bytes

        assert!(file_is_complete(&path, 5), "exact size matches");
        assert!(!file_is_complete(&path, 4), "wrong size rejected");
        assert!(
            !file_is_complete(&dir.join("missing.gguf"), 5),
            "missing file rejected"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
