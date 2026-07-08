# Selection → Review Overlay — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.
> Design doc: [`2026-07-07-selection-review-overlay-design.md`](./2026-07-07-selection-review-overlay-design.md).
> Work in the worktree `.worktrees/selection-review-overlay` (branch `feat/selection-review-overlay`).

**Goal:** A rebindable global hotkey grabs the current selection and pops an always-on-top card near
the cursor showing the offline Proofread review (Harper) with AI action chips; Apply pastes the result
back into the source field with a single-`Ctrl+Z`-revert. Then a Phase B adds a bundled local Gemma
E2B model so the AI chips work fully offline.

**Architecture:** Reuse the existing cleanup-hotkey copy phase (`actions.rs`) and the existing
`ReviewPanel` + `analyze_text` (Harper). Add one focusable, always-on-top Tauri window (created
programmatically like the recording overlay) positioned at the cursor. Route selected text in via
`emit_to`, route the accepted result back via a new command that refocuses the saved foreground window
and pastes via `clipboard::inject_bulk`. AI actions route through the existing
`post_process_is_configured` seam so Phase B's local model slots in behind it with no overlay rework.

**Tech Stack:** Rust + Tauri 2, `windows` crate (Win32 focus APIs — already a dependency), enigo,
React/TypeScript, Harper (offline grammar). Phase B: `llama-cpp-2` + GGUF (reuses the shipped GGML stack).

**Build/run/test reminders (from handoff — do not re-derive):**
- Build: `cd src-tauri && export CARGO_TARGET_DIR="C:/dtfb" && export PATH="$HOME/.cargo/bin:$PATH" && cargo build`
- Rust tests: `cd src-tauri && export CARGO_TARGET_DIR="C:/dtfb" && cargo test --lib`
- Frontend: `node_modules/.bin/tsc --noEmit -p tsconfig.json`; `node_modules/.bin/eslint <files>`
- **`taskkill //F //IM dotflow.exe`** before any rebuild/relaunch (single-instance forwarding + DLL lock).
- specta `bindings.ts` regenerates only on a real app run → **hand-add** new commands/types.
- Run built binary: `"C:/dtfb/debug/dotflow.exe" --debug >/tmp/x.log 2>&1 &` (connects to Vite `:1420`).

---

# PHASE A — The overlay (v1, ships without a bundled LLM)

## Task A1: Extract a shared `copy_selection` helper (DRY the copy phase)

The copy phase is currently inline in `cleanup_selection` (`src-tauri/src/actions.rs:930-1013`). Both
the cleanup hotkey and the new review hotkey need it, so extract it verbatim into one function. This is
an I/O-bound OS refactor (synthetic keys + clipboard) — **not unit-testable honestly**; it is verified
by the existing cleanup hotkey continuing to work unchanged (Task A11 exercise) plus a green build.

**Files:**
- Modify: `src-tauri/src/actions.rs` (extract from `cleanup_selection`, lines 930-1013)

**Step 1:** Add a new function above `cleanup_selection`, moving the SENTINEL const + Phase-1 block
verbatim. Return `Ok(None)` on the "no selection" bail (after restoring the clipboard), else
`Ok(Some((original, selected)))`:

```rust
/// Copy the current selection using the wait-for-release + clipboard-sentinel dance, restoring the
/// user's clipboard on the "nothing selected" path. Returns (original_clipboard, selected_text) or
/// None when no selection was detected. Shared by the cleanup and review hotkeys.
async fn copy_selection(app: &AppHandle) -> Result<Option<(String, String)>, String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    const SENTINEL: &str = "\u{2063}\u{2063}dotflow-clip-sentinel\u{2063}\u{2063}";

    let app_c = app.clone();
    let (original, selected) = tauri::async_runtime::spawn_blocking(move || {
        // ... verbatim body of the existing Phase 1 block (actions.rs:939-985) ...
        (original, selected)
    })
    .await
    .map_err(|e| format!("copy task failed: {e}"))?;

    if selected.trim().is_empty() || selected == SENTINEL {
        debug!("copy_selection: no selection detected");
        let app_c = app.clone();
        let orig = original.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let _ = app_c.clipboard().write_text(&orig);
        })
        .await;
        return Ok(None);
    }
    Ok(Some((original, selected)))
}
```

**Step 2:** Rewrite `cleanup_selection` to call it:

```rust
async fn cleanup_selection(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let Some((original, selected)) = copy_selection(app).await? else { return Ok(()); };
    let settings = get_settings(app);
    let cleaned = resolve_cleanup(&settings, &selected).await;
    let app_c = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if cleaned != selected && !cleaned.trim().is_empty() {
            if let Err(e) = crate::clipboard::inject_bulk(&cleaned, &app_c) {
                warn!("Cleanup hotkey: paste failed: {e}");
            }
        }
        let _ = app_c.clipboard().write_text(&original);
    })
    .await
    .map_err(|e| format!("cleanup paste task failed: {e}"))?;
    Ok(())
}
```

**Step 3:** Build. Run: `cd src-tauri && export CARGO_TARGET_DIR="C:/dtfb" && cargo build --bin dotflow` — Expected: compiles clean.

**Step 4:** Commit: `git commit -m "refactor(actions): extract shared copy_selection helper"`

---

## Task A2: Pure overlay-position clamping (unit-tested — the teeth of Phase A)

A pure geometry function that anchors the card below-right of the cursor and flips/clamps so it never
leaves the work area. This is the one genuinely unit-testable piece; give it real boundary + would-be-
off-screen assertions.

**Files:**
- Create: `src-tauri/src/dotflow/overlay_pos.rs`
- Modify: `src-tauri/src/dotflow/mod.rs` (add `pub mod overlay_pos;`)

**Step 1: Write the failing test** (put tests in the same file):

```rust
#[derive(Clone, Copy, Debug)]
pub struct WorkArea { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }

#[cfg(test)]
mod tests {
    use super::*;
    const FHD: WorkArea = WorkArea { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0 };

    #[test]
    fn sits_below_right_of_cursor_mid_screen() {
        // GAP = 12; no flip needed away from edges.
        assert_eq!(clamp_overlay_position((500.0, 500.0), (420.0, 300.0), FHD), (512.0, 512.0));
    }

    #[test]
    fn flips_left_at_right_edge() {
        // 1900+12+420 = 2332 > 1920 -> flip left: 1900-12-420 = 1468
        let (x, _) = clamp_overlay_position((1900.0, 500.0), (420.0, 300.0), FHD);
        assert_eq!(x, 1468.0);
    }

    #[test]
    fn flips_up_at_bottom_edge() {
        // 1070+12+300 = 1382 > 1080 -> flip up: 1070-12-300 = 758
        let (_, y) = clamp_overlay_position((500.0, 1070.0), (420.0, 300.0), FHD);
        assert_eq!(y, 758.0);
    }

    #[test]
    fn never_leaves_work_area_when_even_a_flip_overflows() {
        // Tiny work area, window bigger than the gap allows either way: must clamp to the edge,
        // NOT return a negative/off-screen coord. Fails if clamping is removed.
        let tiny = WorkArea { x: 0.0, y: 0.0, width: 400.0, height: 300.0 };
        let (x, y) = clamp_overlay_position((10.0, 10.0), (420.0, 300.0), tiny);
        assert!(x >= tiny.x && x + 420.0 <= tiny.width + 1.0, "x off-screen: {x}");
        assert!(y >= tiny.y, "y off-screen: {y}");
        assert_eq!((x, y), (0.0, 0.0));
    }
}
```

**Step 2: Run to verify it fails.** Run: `cd src-tauri && export CARGO_TARGET_DIR="C:/dtfb" && cargo test --lib overlay_pos` — Expected: FAIL (`clamp_overlay_position` not found).

**Step 3: Implement:**

```rust
/// Top-left position for the review overlay: below-right of the cursor by a gap, flipping to
/// above/left near the right/bottom edges, then hard-clamped so it never leaves the work area.
/// All coordinates share one logical-pixel space.
pub fn clamp_overlay_position(cursor: (f64, f64), win: (f64, f64), work: WorkArea) -> (f64, f64) {
    const GAP: f64 = 12.0;
    let (cx, cy) = cursor;
    let (w, h) = win;
    let mut x = cx + GAP;
    let mut y = cy + GAP;
    if x + w > work.x + work.width { x = cx - GAP - w; }
    if y + h > work.y + work.height { y = cy - GAP - h; }
    x = x.clamp(work.x, (work.x + work.width - w).max(work.x));
    y = y.clamp(work.y, (work.y + work.height - h).max(work.y));
    (x, y)
}
```

**Step 4: Run to verify pass.** Run: `cargo test --lib overlay_pos` — Expected: 4 passed.

**Step 5: Commit:** `git commit -m "feat(overlay): pure cursor-anchored position clamping + tests"`

---

## Task A3: Settings — `selection_review_enabled` bool + `review_selection` binding

**Files:**
- Modify: `src-tauri/src/settings.rs`

**Step 1: Write the failing test** (append to the existing `#[cfg(test)]` module, or add one):

```rust
#[test]
fn defaults_include_review_selection_binding_and_flag() {
    let s = get_default_settings();
    assert!(s.selection_review_enabled, "review must default ON");
    let b = s.bindings.get("review_selection").expect("review_selection binding missing");
    assert_eq!(b.id, "review_selection");
    // Must carry a modifier and must NOT be Ctrl+Alt (AltGr) — see validator rule.
    assert!(b.default_binding.contains("shift") || b.default_binding.contains("ctrl"));
    assert!(!(b.default_binding.contains("ctrl") && b.default_binding.contains("alt")),
        "Ctrl+Alt is AltGr on Windows");
}
```

**Step 2: Run to verify it fails.** Run: `cargo test --lib defaults_include_review_selection` — Expected: FAIL (no field / no binding).

**Step 3: Implement** — three edits:

a) Add the struct field (in `AppSettings`, near `post_process_enabled` ~line 418):
```rust
    #[serde(default = "default_selection_review_enabled")]
    pub selection_review_enabled: bool,
```
b) Add the default fn (near `default_post_process_enabled`, ~line 585):
```rust
fn default_selection_review_enabled() -> bool { true }
```
c) Add the literal in the `AppSettings { ... }` block (~line 901):
```rust
        selection_review_enabled: default_selection_review_enabled(),
```
d) Add the binding in `get_default_settings()` (after the `cleanup_selection` block, ~line 862):
```rust
    #[cfg(target_os = "macos")]
    let default_review_shortcut = "cmd+shift+j";
    #[cfg(not(target_os = "macos"))]
    let default_review_shortcut = "ctrl+shift+j";
    bindings.insert(
        "review_selection".to_string(),
        ShortcutBinding {
            id: "review_selection".to_string(),
            name: "Review Selected Text".to_string(),
            description: "Copies the selected text and opens a floating review card near the cursor \
                to proofread (offline) or run an AI rewrite before pasting it back.".to_string(),
            default_binding: default_review_shortcut.to_string(),
            current_binding: default_review_shortcut.to_string(),
        },
    );
```

**Step 4: Run to verify pass.** Run: `cargo test --lib defaults_include_review_selection` — Expected: PASS.

**Step 5:** Gate registration on the flag — in `src-tauri/src/shortcut/tauri_impl.rs::init_shortcuts`
(~line 27, mirror the `transcribe_with_post_process` skip):
```rust
        if id == "review_selection" && !user_settings.selection_review_enabled { continue; }
```

**Step 6: Commit:** `git commit -m "feat(settings): review_selection binding + selection_review_enabled flag"`

---

## Task A4: `change_selection_review_enabled` command + frontend settings plumbing

**Files:**
- Modify: `src-tauri/src/shortcut/mod.rs` (add change command, mirror existing `change_*_setting`)
- Modify: `src-tauri/src/lib.rs:661` (register in `collect_commands!`)
- Modify: `src/bindings.ts` (hand-add command + `AppSettings` field)
- Modify: `src/stores/settingsStore.ts:76-168` (add `settingUpdaters` entry)

**Step 1:** Add the Rust command (model on the typed-expander change setter in `shortcut/mod.rs`):
```rust
#[tauri::command]
#[specta::specta]
pub fn change_selection_review_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = crate::settings::load_or_create_app_settings(&app);
    settings.selection_review_enabled = enabled;
    crate::settings::write_settings(&app, &settings)?; // use whatever the sibling setters call
    // Re-init shortcuts so the binding registers/unregisters immediately:
    crate::shortcut::tauri_impl::reregister_all(&app); // or the existing re-init entry point
    Ok(())
}
```
> NOTE: match the EXACT persistence + re-register calls the neighbouring `change_typed_expander_setting`
> uses (read that function first). If it emits a settings-changed event, do the same.

**Step 2:** Register in `collect_commands!` (`lib.rs:661`, beside the cleanup commands):
```rust
            commands::cleanup::post_process_is_configured,
            shortcut::change_selection_review_enabled_setting,
```

**Step 3:** Hand-add to `src/bindings.ts`:
- In the `commands` object (beside `postProcessIsConfigured`):
```ts
async changeSelectionReviewEnabledSetting(enabled: boolean) : Promise<Result<null, string>> {
    try { return { status: "ok", data: await TAURI_INVOKE("change_selection_review_enabled_setting", { enabled }) }; }
    catch (e) { if (e instanceof Error) throw e; else return { status: "error", error: e as any }; }
},
```
- In the `AppSettings` type (starts ~line 927): add `selection_review_enabled: boolean`.

**Step 4:** Add the `settingUpdaters` entry (`settingsStore.ts`, beside `experimental_typed_expander`):
```ts
  selection_review_enabled: (value) =>
    commands.changeSelectionReviewEnabledSetting(value as boolean),
```

**Step 5:** Build + typecheck. Run: `cargo build --bin dotflow` then `node_modules/.bin/tsc --noEmit -p tsconfig.json` — Expected: both clean.

**Step 6: Commit:** `git commit -m "feat(settings): wire selection_review_enabled toggle end-to-end"`

---

## Task A5: Create the review overlay window (Rust + Vite entry + HTML skeleton)

Use ONE `WebviewWindowBuilder` for all platforms (unlike the recording overlay's NSPanel split) — the
review card is *focusable*, so a normal webview window is correct everywhere. macOS polish deferred.

**Files:**
- Modify: `src-tauri/src/overlay.rs` (add `create_review_overlay`, `show_review_overlay`, `hide_review_overlay`)
- Modify: `src-tauri/src/lib.rs:326` (call `utils::create_review_overlay(app_handle);`)
- Create: `src/overlay/review/index.html`, `src/overlay/review/main.tsx`, `src/overlay/review/ReviewOverlay.tsx`
- Modify: `vite.config.ts:20-28` (add entry)

**Step 1:** Vite entry (`vite.config.ts`):
```ts
        overlay: resolve(__dirname, "src/overlay/index.html"),
        reviewOverlay: resolve(__dirname, "src/overlay/review/index.html"),
```

**Step 2:** `src/overlay/review/index.html` — copy `src/overlay/index.html` but title "Review" and
`src="/src/overlay/review/main.tsx"`. Body background: use a solid panel (NOT transparent) since this
is an opaque card — set `background: transparent` on html/body and let the React root paint the card.

**Step 3:** `main.tsx` renders `<ReviewOverlay />` (mirror `src/overlay/main.tsx`, import `@/i18n`).
`ReviewOverlay.tsx` starts as a stub that renders "review overlay" (fleshed out in A8).

**Step 4:** Rust window creator in `overlay.rs` (label `"review_overlay"`):
```rust
pub const REVIEW_WIDTH: f64 = 420.0;
pub const REVIEW_HEIGHT: f64 = 340.0;

pub fn create_review_overlay(app_handle: &AppHandle) {
    let mut builder = WebviewWindowBuilder::new(
        app_handle, "review_overlay",
        tauri::WebviewUrl::App("src/overlay/review/index.html".into()),
    )
    .title("Review")
    .resizable(false)
    .inner_size(REVIEW_WIDTH, REVIEW_HEIGHT)
    .shadow(false)
    .maximizable(false).minimizable(false).closable(false)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .transparent(true)
    .focusable(true)     // <-- KEY difference vs recording overlay
    .visible(false);
    if let Some(data_dir) = crate::portable::data_dir() {
        builder = builder.data_directory(data_dir.join("webview"));
    }
    if let Err(e) = builder.build() { error!("Failed to create review overlay: {e}"); }
}
```

**Step 5:** `show_review_overlay(app, text, work_area)` — set size, compute pos via
`clamp_overlay_position` (feed cursor from `input::get_cursor_position` + monitor bounds from
`get_monitor_with_cursor`), `set_position`, `show`, `force_overlay_topmost` (Windows), then
`emit_to("review_overlay", "review-text", ReviewPayload { text, ai_available })`. `hide_review_overlay`
mirrors `hide_recording_overlay` (emit `"review-hide"`, then hide).

**Step 6:** Wire creation at `lib.rs:326`: `utils::create_review_overlay(app_handle);`

**Step 7:** Build. Run: `cargo build --bin dotflow` — Expected: clean. (Window not shown yet.)

**Step 8: Commit:** `git commit -m "feat(overlay): review overlay window scaffold (Rust + Vite entry)"`

---

## Task A6: `ReviewSelectionAction` — copy selection, capture foreground, show overlay

**Files:**
- Modify: `src-tauri/src/actions.rs` (new action + `ACTION_MAP` entry)
- Modify: `src-tauri/src/input.rs` (add `get_foreground_window()` / `set_foreground_window(hwnd)`)
- Modify: `src-tauri/src/lib.rs` (register a `Mutex<Option<isize>>` state for the saved HWND)

**Step 1:** Add Win32 focus helpers to `input.rs` (feature `Win32_UI_WindowsAndMessaging` already on):
```rust
#[cfg(target_os = "windows")]
pub fn get_foreground_window() -> Option<isize> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() { None } else { Some(hwnd.0 as isize) }
}
#[cfg(not(target_os = "windows"))]
pub fn get_foreground_window() -> Option<isize> { None }

#[cfg(target_os = "windows")]
pub fn set_foreground_window(hwnd: isize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
    use windows::Win32::Foundation::HWND;
    unsafe { SetForegroundWindow(HWND(hwnd as *mut _)).as_bool() }
}
#[cfg(not(target_os = "windows"))]
pub fn set_foreground_window(_hwnd: isize) -> bool { false }
```
> Verify `HWND(hwnd as *mut _)` against the crate's 0.61 HWND repr (the fg example at
> `typed_expander/backend.rs:320` shows the in-use form — match it).

**Step 2:** Saved-HWND state. In `lib.rs` setup add `app.manage(SavedForegroundWindow(Mutex::new(None)))`
(define `pub struct SavedForegroundWindow(pub Mutex<Option<isize>>);`).

**Step 3:** The action:
```rust
struct ReviewSelectionAction;
impl ShortcutAction for ReviewSelectionAction {
    fn start(&self, app: &AppHandle, _b: &str, _s: &str) {
        log::info!("Review-selection hotkey fired");   // disambiguates fired-vs-panicked
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = review_selection(&app).await { warn!("Review selection failed: {e}"); }
        });
    }
    fn stop(&self, _a: &AppHandle, _b: &str, _s: &str) {}
}

async fn review_selection(app: &AppHandle) -> Result<(), String> {
    // Capture the field we came from BEFORE anything can steal focus.
    let fg = crate::input::get_foreground_window();
    if let Some(state) = app.try_state::<crate::SavedForegroundWindow>() {
        *state.0.lock().unwrap() = fg;
    }
    let Some((_original, selected)) = copy_selection(app).await? else { return Ok(()); };
    // NOTE: we intentionally do NOT restore the clipboard here — apply_review_result restores it
    // after pasting. Stash `_original` in state alongside the HWND if you want restore-on-cancel.
    let ai_available = crate::commands::cleanup::post_process_is_configured(app.clone());
    crate::overlay::show_review_overlay(app, &selected, ai_available)?;
    Ok(())
}
```
Register in `ACTION_MAP` (actions.rs:1053):
```rust
    map.insert("review_selection".to_string(),
        Arc::new(ReviewSelectionAction) as Arc<dyn ShortcutAction>);
```

**Step 4:** Build. Run: `cargo build --bin dotflow` — Expected: clean.

**Step 5:** Manual smoke (deferred to A11 full exercise): the card should now appear at the cursor with
the copied text emitted. Commit: `git commit -m "feat(actions): review_selection hotkey opens the overlay"`

---

## Task A7: `apply_review_result` command — refocus source + paste back

**Files:**
- Modify: `src-tauri/src/commands/cleanup.rs` (add command)
- Modify: `src-tauri/src/lib.rs:661` (register)
- Modify: `src/bindings.ts` (hand-add binding)

**Step 1:** Command:
```rust
/// Paste a reviewed result back into the field the review hotkey was fired from. Refocuses the saved
/// foreground window, waits for it to actually regain focus, then pastes via inject_bulk. Also hides
/// the overlay.
#[tauri::command]
#[specta::specta]
pub async fn apply_review_result(app: AppHandle, text: String) -> Result<(), String> {
    crate::overlay::hide_review_overlay(&app);
    let hwnd = app.try_state::<crate::SavedForegroundWindow>()
        .and_then(|s| *s.0.lock().unwrap());
    let app_c = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(hwnd) = hwnd {
            crate::input::set_foreground_window(hwnd);
            // Poll until it actually regains foreground (don't guess) — up to ~500ms.
            for _ in 0..25 {
                if crate::input::get_foreground_window() == Some(hwnd) { break; }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            log::info!("apply_review_result: refocused source hwnd={hwnd}"); // focus checkpoint
        }
        if !text.trim().is_empty() {
            if let Err(e) = crate::clipboard::inject_bulk(&text, &app_c) {
                warn!("apply_review_result: paste failed: {e}");
            }
        }
    }).await.map_err(|e| format!("apply task failed: {e}"))?;
    Ok(())
}
```
Also add `pub async fn cancel_review(app: AppHandle) { crate::overlay::hide_review_overlay(&app); }`
(command) for Esc/close.

**Step 2:** Register both in `collect_commands!` (`lib.rs:661`).

**Step 3:** Hand-add to `bindings.ts` `commands` (both return `Result<null,string>`, arg `{ text }` / none).

**Step 4:** Build + typecheck. Expected: clean. Commit:
`git commit -m "feat(commands): apply_review_result refocuses source and pastes back"`

---

## Task A8: Review overlay React UI — chips + ReviewPanel + Apply/Copy/Close

**Files:**
- Modify: `src/overlay/review/ReviewOverlay.tsx`
- Reuse: `src/components/settings/cleanup/ReviewPanel.tsx` (props `{ text, onResult }`)

**Step 1:** State + listeners. Listen for `"review-text"` (payload `{ text, ai_available }`) and
`"review-hide"`. Keep `text`, `aiAvailable`, `activeAction` (`"proofread" | "rewrite" | "formal" |
"summarize"`, default `"proofread"`), `reviewResult` (from `ReviewPanel.onResult`).

```tsx
import { listen } from "@tauri-apps/api/event";
import { commands } from "@/bindings";
import { ReviewPanel } from "@/components/settings/cleanup/ReviewPanel";

// on mount:
const un1 = await listen("review-text", (e) => {
  const p = e.payload as { text: string; ai_available: boolean };
  setText(p.text); setAiAvailable(p.ai_available); setActiveAction("proofread");
});
const un2 = await listen("review-hide", () => { /* clear */ });
```

**Step 2:** Chip row. `Proofread ·offline` always enabled; `Rewrite/Formal/Summarize` disabled when
`!aiAvailable` (with a tooltip/hint "Configure AI in Settings → Cleanup"). Selecting Proofread renders
`<ReviewPanel text={text} onResult={setReviewResult} />`. AI chips are wired for real in Phase B; in
Phase A they are visible-but-disabled (no local model yet, and if Ollama is configured they can call
`previewCleanup` as an interim — OPTIONAL, keep simple: leave disabled unless `aiAvailable`).

**Step 3:** Footer. `Apply` → `commands.applyReviewResult(reviewResult ?? text)`. `Copy` →
`navigator.clipboard.writeText(reviewResult ?? text)`. `Close` → `commands.cancelReview()`.
Keyboard: `Enter` = Apply, `Escape` = Close (window-level `keydown`).

**Step 4:** Styling — opaque card using the design tokens (surface-ladder: white panel, green-biased
hairline, no shadow), `~420px`, `max-height` with internal scroll. Match `theme.css`.

**Step 5:** Typecheck + lint. Run: `node_modules/.bin/tsc --noEmit -p tsconfig.json` and
`node_modules/.bin/eslint src/overlay/review` — Expected: clean (ReviewPanel is English-only already;
add i18n keys for the overlay chrome in A10, or use inline defaults + eslint-disable consistent with
ReviewPanel).

**Step 6: Commit:** `git commit -m "feat(overlay): review card UI — chips, ReviewPanel, apply/copy/close"`

---

## Task A9: Settings UI — master toggle + rebindable hotkey in Cleanup section

**Files:**
- Modify: `src/components/settings/cleanup/CleanupSettings.tsx`

**Step 1:** Add a master toggle mirroring `TypedExpander.tsx` (uses `useSettings`):
```tsx
const { getSetting, updateSetting, isUpdating } = useSettings();
const reviewEnabled = getSetting("selection_review_enabled") ?? true;
// ...
<ToggleSwitch
  checked={reviewEnabled}
  onChange={(v) => updateSetting("selection_review_enabled", v)}
  isUpdating={isUpdating("selection_review_enabled")}
  label={t("settings.review.enable", "Selection review overlay")}
  description={t("settings.review.enableDesc", "A hotkey opens a floating review card near the cursor. Requires AI configured for Rewrite/Formal/Summarize.")}
  grouped
/>
```

**Step 2:** Add the rebindable hotkey control (only meaningful when enabled):
```tsx
{reviewEnabled && <ShortcutInput shortcutId="review_selection" grouped />}
```

**Step 3:** Typecheck + lint. Expected: clean. Commit:
`git commit -m "feat(settings-ui): selection-review toggle + hotkey in Cleanup"`

---

## Task A10: i18n keys across all 21 locales

**Files:**
- Modify: `src/i18n/locales/*/translation.json` (all 21)

**Step 1:** Add the new keys (`settings.review.enable`, `settings.review.enableDesc`, and any overlay
chrome keys you localized) to `en/translation.json` first, then propagate to all 21 locales (the CI
translation check requires every key in every locale). Use the repo's add-key script if present;
otherwise add English values to each as a fallback and mark for translation.

**Step 2:** Verify: `bun run` the translation-completeness check the CI uses (see `FORK.md`/CI). Expected:
no missing keys. Commit: `git commit -m "i18n: selection-review keys across all locales"`

---

## Task A11: End-to-end verification (exercise, not a fake test) + open PR-less finish

This is where the OS-boundary behavior is *proven* (the design's non-unit-testable claims).

**Step 1:** `taskkill //F //IM dotflow.exe` (ensure none running). Rebuild: `cargo build --bin dotflow`.
Ensure Vite dev server is running (`bun run dev`), launch `"C:/dtfb/debug/dotflow.exe" --debug >/tmp/x.log 2>&1 &`.

**Step 2:** Use the `verify` skill / manual drive. Confirm, watching real behavior:
- Select text in Notepad/browser, press `Ctrl+Shift+J` → card appears **at the cursor**, showing the
  Proofread result with issues underlined.
- Accept a fix, press `Apply` → text is replaced in the source field; a **single `Ctrl+Z` reverts** it.
- `Esc` closes with no change. AI chips are **disabled** (no model yet / AI not configured).
- Toggle `selection_review_enabled` off in Settings → hotkey no longer fires. Rebinding works.
- Grep the debug log for the `Review-selection hotkey fired` and `apply_review_result: refocused` lines
  to confirm fire + refocus actually happened (per the silent-async-panic + focus gotchas).

**Step 3:** Run full test + lint gate: `cargo test --lib` (all pass incl. new), `tsc --noEmit`,
`eslint`, `prettier --check`. Fix anything red.

**Step 4:** Finish Phase A per superpowers:finishing-a-development-branch (merge to main or open PR —
following AGENTS.md's PR template if a PR). **Phase A is independently shippable.**

---

# PHASE B — Local Gemma E2B backend (AI chips work fully offline)

> Phase B has genuine integration unknowns (a C++ inference crate on Windows, model download). It starts
> with a spike; the later task granularity is refined after B1 confirms the build. All AI actions route
> through the existing `post_process`/`resolve_cleanup` seam, so no Phase A overlay code changes.

## Task B1: Spike — add `llama-cpp-2`, confirm it builds & links on Windows

**Files:** `src-tauri/Cargo.toml`; a throwaway `examples/llama_smoke.rs`.

**Step 1:** `cargo add llama-cpp-2` (utilityai crate — tracks upstream, static-links GGML). Confirm it
reuses/does-not-conflict-with the existing GGML from `transcribe-cpp`.

**Step 2:** Write a tiny example that loads a small GGUF and generates 10 tokens from a fixed prompt.

**Step 3:** Build with `CARGO_TARGET_DIR="C:/dtfb"`. Expected: compiles + links (this is the real risk).
If it fails (GGML symbol clash / CMake), STOP and report — resolve before proceeding.

**Step 4:** Commit the spike behind a feature flag `local-llm` (default off) so it can't break Phase A CI:
`git commit -m "spike(local-llm): llama-cpp-2 builds and generates on Windows"`

## Task B2: Gemma E2B model in the model manager (download-on-demand)

**Files:** `src-tauri/src/managers/model.rs` (`DOTFLOW_MODEL_REPOS`).

Add a Gemma-4 E2B **Q4_K_M GGUF** entry (Apache-2.0; verify the exact repo/quant on HF first — see the
design doc's Future-AI table). Reuse the existing downloader + progress UI. Do **not** bundle in the
installer — fetch on first AI use. Commit.

## Task B3: Local inference module behind a trait

**Files:** Create `src-tauri/src/dotflow/local_llm.rs`.

Define a `TextTransformer` trait (`async fn transform(&self, system: &str, input: &str) -> Result<String>`).
Implement it with `llama-cpp-2`: load model once (lazy, cached in state), apply Gemma's chat template,
low temperature, cap output length, `catch_unwind` around inference (silent-async-panic gotcha). Unit-
test the **prompt/template assembly** (pure string function — real assertions on the formatted prompt),
not the generation. Commit.

## Task B4: Route the AI seam to the local model

**Files:** `src-tauri/src/actions.rs` (`resolve_cleanup` / `cleanup_with_llm`),
`src-tauri/src/commands/cleanup.rs` (`post_process_is_configured`).

Extend the post-process resolution so the order is: configured cloud/Ollama LLM → **local Gemma (if
downloaded)** → Harper → deterministic. Make `post_process_is_configured` (which drives the overlay's
`ai_available`) return true when the local model is present. Now the overlay's AI chips light up offline.
Commit.

## Task B5: Per-action prompts + wire the overlay AI chips

**Files:** `src-tauri/src/commands/cleanup.rs` (new `ai_transform(text, action)` command),
`src/overlay/review/ReviewOverlay.tsx`, `src/bindings.ts`.

Add system prompts for `rewrite`, `formal`, `summarize` (+ a deeper proofread). Command
`ai_transform(text, action) -> Result<String,String>`. Overlay AI chips call it, show a spinner while it
runs, render a before→after result view with the same `Apply`/`Copy`. Register + hand-add bindings.
Commit.

## Task B6: Verify Phase B end-to-end

Exercise: download the model, run each AI action offline on selected text, confirm Apply pastes back and
`Ctrl+Z` reverts. Confirm CPU latency is acceptable for short text (design target: 1.7–2B ≈ 1–5s, 3–4B ≈
3–15s). Run full test/lint gate. Finish per finishing-a-development-branch.

---

## Notes / deferred (do not scope-creep into A or B)

- Post-dictation review (reuse the same overlay window, setting-gated, off by default) — later.
- macOS NSPanel polish for the review window — later (Phase A uses a plain focusable window everywhere).
- ML-GEC grammar tier (ONNX on `ort`) — later rung, independent of this work.
- Medical terminology pack + medical-model guidance — see the design doc's Future-AI section.
- Spoken output (Kokoro) — explicitly off the map.
