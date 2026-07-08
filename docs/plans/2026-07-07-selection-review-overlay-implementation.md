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

## Adversarial-sweep fold (2026-07-07)

This plan was swept by 4 decorrelated skeptics; the DTF gate returned HALT+FOLD on 5 CRITICAL/HIGH.
The findings below are folded into the tasks (each fix tagged `[Fxx]`). Do NOT treat a green build as
proof any of these are resolved — the OS-boundary ones (F1/F2/F3/F5/F6/F9) are proven only by the
Task A11 exercise. Summary of what changed vs the first draft:

- **[F1] CRITICAL** — the review flow now stashes the user's `original` clipboard and **restores it on
  both Apply and Cancel** (it did neither before → silent clipboard destruction every use).
- **[F2/F3] HIGH** — focus is now handled with an explicit **AttachThreadInput force-foreground** helper
  (the default HandyKeys backend grants no `SetForegroundWindow` rights), and Apply **only pastes if
  refocus actually succeeded** (`IsWindow` + foreground check), else it aborts and leaves the result on
  the clipboard + notifies.
- **[F4] HIGH** — Task A4 rewritten to the real `write_settings(&app, settings)` (by value, no `?`) and
  the real single-binding register/unregister (no invented `reregister_all`).
- **[F5] HIGH** — the `selection_review_enabled` gate is applied at **all three** registration sites via
  one shared helper.
- **[F6] MED** — cursor/monitor coords are converted to **logical px (÷ scale_factor)** before the pure
  clamp; the clamp stays pure and gains an origin≠0 test.
- **[F7] MED** — the review overlay entry imports a stylesheet that does `@import "tailwindcss"` so
  ReviewPanel's utilities render.
- **[F8] MED** — one signature: `show_review_overlay(app, text, ai_available)`, work area computed inside.
- **[F9] MED** — a re-entrancy flag ignores the hotkey while the card is open.
- **[F10] MED** — the impossible A2 test assertion is replaced with a satisfiable realistic-clamp case.
- **[F11] MED** — payload is stored in state and **pulled on mount** (plus the emit), so a late listener
  never yields a blank card.
- **[F12] LOW** `lock().ok()` not `unwrap()`. **[F13] LOW** synchronous hide before paste. **[F14] LOW**
  Phase B dep is `optional=true` behind `local-llm` + example `required-features`. **[F15]** "shippable"
  reframed: Phase A ships only after F1 is fixed; AI chips are inert until Phase B.

Shared state introduced by the fold (replaces the HWND-only state in the first draft):

```rust
// src-tauri/src/lib.rs (or a small module) — managed via app.manage(...)
pub struct ReviewContext(pub Mutex<Option<ReviewCtx>>);
pub struct ReviewCtx {
    pub source_hwnd: Option<isize>,   // the field we came from (GetForegroundWindow at fire time)
    pub original_clipboard: String,   // [F1] restore this on apply AND cancel
    pub payload: Option<(String, bool)>, // [F11] (selected_text, ai_available) pulled on mount
}
pub static REVIEW_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false); // [F9]
```

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
    fn clamps_to_edge_when_flip_would_still_overflow() {
        // [F10] Realistic case: window FITS the work area, but anchoring+flipping near the top-left
        // corner would push it off the left/top. Must clamp back to the edge, not return negatives.
        // Fails if the final .clamp() lines are deleted (they return (-427, -302) without them).
        let wa = WorkArea { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };
        let (x, y) = clamp_overlay_position((5.0, 5.0), (420.0, 300.0), wa);
        // default 17,17 fits (17+420=437<=800) so NO flip; stays at 17,17 — assert it's on-screen and
        // exactly the anchored position (catches an unintended flip AND an over-eager clamp).
        assert_eq!((x, y), (17.0, 17.0));
        assert!(x >= wa.x && x + 420.0 <= wa.width, "off right/left edge");
        assert!(y >= wa.y && y + 300.0 <= wa.height, "off top/bottom edge");
    }

    #[test]
    fn clamps_hard_when_window_exceeds_work_area() {
        // Degenerate: window wider than the work area — cannot fully fit, so clamp pins the top-left
        // to the work-area origin (the best we can do). Asserts the pin, NOT an impossible "fully
        // on-screen". Fails (returns negatives) if the clamp lines are removed.
        let tiny = WorkArea { x: 0.0, y: 0.0, width: 400.0, height: 300.0 };
        assert_eq!(clamp_overlay_position((10.0, 10.0), (420.0, 320.0), tiny), (0.0, 0.0));
    }

    #[test]
    fn respects_non_zero_monitor_origin() {
        // [F6] Second monitor at logical origin (1920, 0). Mid-screen cursor must anchor below-right
        // relative to THAT origin, not (0,0). Caller is responsible for passing LOGICAL coords.
        let m2 = WorkArea { x: 1920.0, y: 0.0, width: 1920.0, height: 1080.0 };
        assert_eq!(clamp_overlay_position((2400.0, 500.0), (420.0, 300.0), m2), (2412.0, 512.0));
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

**Step 4: Run to verify pass.** Run: `cargo test --lib overlay_pos` — Expected: **6 passed**.

**Step 5:** Add a doc comment on `clamp_overlay_position` stating **all inputs MUST be logical pixels**
(the caller in Task A5 converts enigo's physical cursor + physical monitor bounds by `÷ scale_factor`
before calling — see `[F6]`). The pure function stays untested-against-DPI on purpose; the conversion
is exercised in the A11 live run.

**Step 6: Commit:** `git commit -m "feat(overlay): pure cursor-anchored position clamping + tests"`

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

**Step 5: [F5] Gate registration at ALL THREE sites** (the first draft patched only one; the default
Windows backend is HandyKeys, so a single gate leaves the hotkey live). Add a shared predicate and use
it everywhere the `transcribe_with_post_process` skip appears:
```rust
// src-tauri/src/shortcut/mod.rs (or wherever is shared)
pub fn review_selection_registrable(id: &str, s: &settings::AppSettings) -> bool {
    !(id == "review_selection" && !s.selection_review_enabled)
}
```
Apply the skip at each site (grep for the existing `transcribe_with_post_process` skip to find them):
- `src-tauri/src/shortcut/tauri_impl.rs:27` (Tauri backend init)
- `src-tauri/src/shortcut/mod.rs:396` (`register_all_shortcuts_for_implementation`, the runtime re-register path)
- `src-tauri/src/shortcut/handy_keys.rs:437` (the **default** HandyKeys backend)

e.g. at each: `if id == "review_selection" && !user_settings.selection_review_enabled { continue; }`
> Verify the exact three line numbers before editing — they may drift. The acceptance test (A11) that
> "toggle off → hotkey no longer fires" MUST be checked on the **default (HandyKeys)** backend, not just Tauri.

**Step 6: Commit:** `git commit -m "feat(settings): review_selection binding + flag, gated at all backends"`

---

## Task A4: `change_selection_review_enabled` command + frontend settings plumbing

**Files:**
- Modify: `src-tauri/src/shortcut/mod.rs` (add change command, mirror existing `change_*_setting`)
- Modify: `src-tauri/src/lib.rs:661` (register in `collect_commands!`)
- Modify: `src/bindings.ts` (hand-add command + `AppSettings` field)
- Modify: `src/stores/settingsStore.ts:76-168` (add `settingUpdaters` entry)

**Step 1: [F4]** Add the Rust command — **read `change_post_process_enabled_setting` in
`shortcut/mod.rs` first and mirror it EXACTLY** (it is the true sibling: a bool that gates one binding).
The first draft's `write_settings(&app, &settings)?` and `reregister_all` were both wrong. Real shapes:
- `pub fn write_settings(app: &AppHandle, settings: AppSettings)` — takes `AppSettings` **by value**,
  returns **`()`** (no `?`). Call: `settings::write_settings(&app, settings);`
- There is **no** `reregister_all`. Register/unregister the **single** `review_selection` binding by name
  using the same helpers the sibling uses (`register_shortcut` / `unregister_shortcut`, backend-aware).

```rust
#[tauri::command]
#[specta::specta]
pub fn change_selection_review_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = crate::settings::load_or_create_app_settings(&app);
    settings.selection_review_enabled = enabled;
    crate::settings::write_settings(&app, settings.clone()); // by value, no `?`

    // Register/unregister ONLY the review_selection binding, exactly like the sibling does for its own.
    if let Some(binding) = settings.bindings.get("review_selection").cloned() {
        if enabled {
            let _ = /* the same register call change_post_process_enabled_setting uses */
                crate::shortcut::register_binding_for_current_backend(&app, binding);
        } else {
            let _ = crate::shortcut::unregister_binding_for_current_backend(&app, &binding);
        }
    }
    Ok(())
}
```
> The exact helper names (`register_binding_for_current_backend` above is a PLACEHOLDER) must be taken
> verbatim from `change_post_process_enabled_setting` — use whatever it calls so this is backend-aware
> (HandyKeys vs Tauri). If that function re-runs `register_all_shortcuts_for_implementation`, do the same
> instead of the single-binding calls. Do NOT invent an API.

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
`src="/src/overlay/review/main.tsx"`. Body background: `transparent` on html/body; the React root paints
the opaque card.

**Step 3: [F7]** `main.tsx` renders `<ReviewOverlay />` (mirror `src/overlay/main.tsx`, import `@/i18n`)
AND **imports a stylesheet that pulls in Tailwind**, because `ReviewPanel` is built from Tailwind v4
utilities (`bg-panel`, `flex`, `px-3`, `decoration-wavy`, …) which are only emitted where
`@import "tailwindcss"` exists — currently only `src/App.css` (the main entry). The recording overlay
dodged this by using hand-written CSS classes; ReviewPanel does NOT. So create
`src/overlay/review/ReviewOverlay.css` containing **both**:
```css
@import "tailwindcss";
@import "../../styles/theme.css";
```
and `import "./ReviewOverlay.css";` in `main.tsx`. Without this the card renders completely unstyled and
`tsc`/`eslint` stay green — the failure only shows at the A11 smoke. `ReviewOverlay.tsx` starts as a stub
(fleshed out in A8).

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

**Step 5: [F8] ONE signature** — `show_review_overlay(app, text: &str, ai_available: bool)`; the work
area is computed **inside**. **[F6]** convert to logical px before the pure clamp; **[F11]** store the
payload in `ReviewContext` and emit; **[F2]** force-foreground the card so keyboard nav works; **[F13]**
use a synchronous hide (see A7). Sketch:
```rust
pub fn show_review_overlay(app: &AppHandle, text: &str, ai_available: bool) -> Result<(), String> {
    let win = app.get_webview_window("review_overlay").ok_or("review overlay missing")?;
    // [F6] enigo cursor + tauri Monitor are PHYSICAL px; the recording overlay divides by scale_factor
    // (overlay.rs:170-184,233-237). Do the same so clamp + LogicalPosition all live in logical space.
    let scale = win.scale_factor().unwrap_or(1.0);
    let (pcx, pcy) = crate::input::get_cursor_position(app).unwrap_or((0, 0));
    let cursor = (pcx as f64 / scale, pcy as f64 / scale);
    let mon = get_monitor_with_cursor(app); // returns a monitor; take .position()/.size() ÷ scale
    let work = WorkArea { /* mon.position()/scale, mon.size()/scale */ };
    let (x, y) = crate::dotflow::overlay_pos::clamp_overlay_position(cursor, (REVIEW_WIDTH, REVIEW_HEIGHT), work);
    let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize { width: REVIEW_WIDTH, height: REVIEW_HEIGHT }));
    let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
    // [F11] persist payload so a late-mounting webview can PULL it (event may fire before listener ready)
    if let Some(state) = app.try_state::<crate::ReviewContext>() {
        if let Ok(mut c) = state.0.lock() {
            if let Some(ctx) = c.as_mut() { ctx.payload = Some((text.to_string(), ai_available)); }
        }
    }
    let _ = win.show();
    #[cfg(target_os = "windows")] force_overlay_topmost(&win);
    // [F2] the default HandyKeys backend confers NO activation rights; explicitly foreground our card so
    // Enter/Esc/arrows work without a click. Use AttachThreadInput-based force_foreground (Task A6 Step 1b).
    #[cfg(target_os = "windows")]
    if let Ok(hwnd) = win.hwnd() { crate::input::force_foreground(hwnd.0 as isize); }
    let _ = win.set_focus();
    // emit as well (fast path when the listener is already up)
    let _ = app.emit_to("review_overlay", "review-text",
        serde_json::json!({ "text": text, "ai_available": ai_available }));
    Ok(())
}
```
**Synchronous hide [F13]:** `hide_review_overlay` must call `win.hide()` **immediately** (NOT the 300ms
deferred pattern of `hide_recording_overlay`) so the always-on-top card is gone before A7 refocuses and
pastes. It also clears `REVIEW_OPEN` (A6/F9) and emits `"review-hide"` for the UI.

**Step 6:** Wire creation at `lib.rs:326`: `utils::create_review_overlay(app_handle);` and manage the new
state: `app.manage(crate::ReviewContext(std::sync::Mutex::new(None)));`

**Step 7:** Build. Run: `cargo build --bin dotflow` — Expected: clean. (Window not shown yet.)

**Step 8: Commit:** `git commit -m "feat(overlay): review overlay window scaffold (Rust + Vite entry)"`

---

## Task A6: `ReviewSelectionAction` — copy selection, capture foreground, show overlay

**Files:**
- Modify: `src-tauri/src/actions.rs` (new action + `ACTION_MAP` entry)
- Modify: `src-tauri/src/input.rs` (add `get_foreground_window` / `set_foreground_window` / `force_foreground`)
- Modify: `src-tauri/src/lib.rs` (manage `ReviewContext` + `REVIEW_OPEN` — see the fold header)

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
> Verify `HWND(hwnd as *mut _)` against the crate's 0.61 HWND repr — the sweep confirmed
> `HWND(pub *mut c_void)` and this cast is correct, matching `typed_expander/backend.rs:320`.

**Step 1b: [F2] `force_foreground(hwnd)`** — the reliable activation the plain `set_foreground_window`
can't guarantee (foreground-lock denies it when we're not already foreground, which is our case under
the HandyKeys hook). Use the standard AttachThreadInput dance: attach our thread to the current
foreground window's thread, `SetForegroundWindow`, then detach. Also add `is_window(hwnd) -> bool`
(wrapping `IsWindow`) for A7's validity guard.
```rust
#[cfg(target_os = "windows")]
pub fn force_foreground(target: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow, IsWindow, BringWindowToTop,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{AttachThreadInput};
    unsafe {
        let hwnd = HWND(target as *mut _);
        if !IsWindow(Some(hwnd)).as_bool() { return false; }
        let fg = GetForegroundWindow();
        let cur = GetCurrentThreadId();
        let fg_thread = GetWindowThreadProcessId(fg, None);
        let _ = AttachThreadInput(cur, fg_thread, true);
        let _ = BringWindowToTop(hwnd);
        let ok = SetForegroundWindow(hwnd).as_bool();
        let _ = AttachThreadInput(cur, fg_thread, false);
        ok
    }
}
#[cfg(not(target_os = "windows"))]
pub fn force_foreground(_target: isize) -> bool { false }

#[cfg(target_os = "windows")]
pub fn is_window(hwnd: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;
    unsafe { IsWindow(Some(HWND(hwnd as *mut _))).as_bool() }
}
#[cfg(not(target_os = "windows"))]
pub fn is_window(_hwnd: isize) -> bool { true }
```
> `AttachThreadInput` needs the `Win32_System_Threading` feature — already enabled (Cargo.toml). Confirm
> `IsWindow`/`AttachThreadInput` arg shapes against the 0.61 API before building; adjust if needed.

**Step 2:** Manage state in `lib.rs` (per the fold header): `app.manage(crate::ReviewContext(Mutex::new(None)));`.
`REVIEW_OPEN` is a module static, no manage needed.

**Step 3: The action — with re-entrancy guard [F9], clipboard stashing [F1], safe locks [F12]:**
```rust
struct ReviewSelectionAction;
impl ShortcutAction for ReviewSelectionAction {
    fn start(&self, app: &AppHandle, _b: &str, _s: &str) {
        log::info!("Review-selection hotkey fired");   // disambiguates fired-vs-panicked
        // [F9] ignore re-fire while the card is open (else the 2nd fire captures the OVERLAY as source).
        if crate::REVIEW_OPEN.swap(true, std::sync::atomic::Ordering::SeqCst) {
            log::info!("Review overlay already open — ignoring re-fire");
            return;
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = review_selection(&app).await {
                warn!("Review selection failed: {e}");
                crate::REVIEW_OPEN.store(false, std::sync::atomic::Ordering::SeqCst); // release on error
            }
        });
    }
    fn stop(&self, _a: &AppHandle, _b: &str, _s: &str) {}
}

async fn review_selection(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    // Capture the field we came from BEFORE anything can steal focus.
    let source_hwnd = crate::input::get_foreground_window();
    // copy_selection returns (original_clipboard, selected); we KEEP original for restore [F1].
    let Some((original, selected)) = copy_selection(app).await? else {
        crate::REVIEW_OPEN.store(false, std::sync::atomic::Ordering::SeqCst); // no selection → release
        return Ok(());
    };
    // Stash context so apply/cancel can restore the clipboard and refocus the right window.
    if let Some(state) = app.try_state::<crate::ReviewContext>() {
        if let Ok(mut c) = state.0.lock() {   // [F12] .ok() form — no unwrap()
            *c = Some(crate::ReviewCtx { source_hwnd, original_clipboard: original, payload: None });
        }
    }
    let ai_available = crate::commands::cleanup::post_process_is_configured(app.clone());
    crate::overlay::show_review_overlay(app, &selected, ai_available)?;
    Ok(())
}
```
> NOTE: `REVIEW_OPEN` is cleared in `hide_review_overlay` (A5 Step 5) which both apply and cancel call —
> so a completed or cancelled review re-arms the hotkey.
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

**Step 1: Command — with GUARDED paste [F3], clipboard restore [F1], synchronous hide [F13], safe locks [F12]:**
```rust
/// Paste a reviewed result back into the field the review hotkey was fired from. Refocuses the saved
/// window; ONLY pastes if the refocus actually succeeded (else the result would land in the wrong app);
/// then restores the user's original clipboard. Hides the overlay first (synchronously).
#[tauri::command]
#[specta::specta]
pub async fn apply_review_result(app: AppHandle, text: String) -> Result<(), String> {
    crate::overlay::hide_review_overlay(&app); // [F13] synchronous hide — card gone before we paste

    // Take the whole context (hwnd + original clipboard), clearing it so a stray second apply no-ops.
    let ctx = app.try_state::<crate::ReviewContext>()
        .and_then(|s| s.0.lock().ok().and_then(|mut c| c.take())); // [F12] .ok(), no unwrap
    let app_c = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let hwnd = ctx.as_ref().and_then(|c| c.source_hwnd);
        // [F3] refocus, then GUARD: only paste if the source window is real AND actually foreground.
        let mut refocused = false;
        if let Some(hwnd) = hwnd {
            if crate::input::is_window(hwnd) {
                crate::input::force_foreground(hwnd);
                for _ in 0..25 { // poll up to ~500ms
                    if crate::input::get_foreground_window() == Some(hwnd) { refocused = true; break; }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        }
        log::info!("apply_review_result: refocused={refocused} hwnd={hwnd:?}"); // focus checkpoint

        if refocused && !text.trim().is_empty() {
            if let Err(e) = crate::clipboard::inject_bulk(&text, &app_c) {
                warn!("apply_review_result: paste failed: {e}");
            }
        } else if !refocused {
            // [F3] Do NOT blind-paste into the wrong window. Leave the result on the clipboard so the
            // user can paste it manually, and tell them.
            use tauri_plugin_clipboard_manager::ClipboardExt;
            let _ = app_c.clipboard().write_text(&text);
            warn!("apply_review_result: could not refocus source — result left on clipboard for manual paste");
            // (Optional: emit a toast/notification event the main window shows.)
            return; // skip the original-clipboard restore below: the result IS the clipboard now
        }

        // [F1] restore the user's ORIGINAL clipboard (inject_bulk left the result/selection on it).
        if let Some(c) = ctx {
            use tauri_plugin_clipboard_manager::ClipboardExt;
            let _ = app_c.clipboard().write_text(&c.original_clipboard);
        }
    }).await.map_err(|e| format!("apply task failed: {e}"))?;
    Ok(())
}
```
Cancel/close — **must also restore the clipboard [F1]** (the copy phase left `selected` on it):
```rust
#[tauri::command]
#[specta::specta]
pub fn cancel_review(app: AppHandle) {
    crate::overlay::hide_review_overlay(&app); // clears REVIEW_OPEN
    if let Some(ctx) = app.try_state::<crate::ReviewContext>()
        .and_then(|s| s.0.lock().ok().and_then(|mut c| c.take())) {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        let _ = app.clipboard().write_text(&ctx.original_clipboard); // [F1]
    }
}
```
> [F11] Also add `get_pending_review(app) -> Option<(String, bool)>` that returns `ctx.payload.clone()`
> so the overlay can PULL its text on mount if it missed the emit (see A8).

**Step 2:** Register all three (`apply_review_result`, `cancel_review`, `get_pending_review`) in
`collect_commands!` (`lib.rs:661`).

**Step 3:** Hand-add to `bindings.ts` `commands` (apply/cancel return `Result<null,string>`;
`get_pending_review` returns `[string, boolean] | null` — unwrapped).

**Step 4:** Build + typecheck. Expected: clean. Commit:
`git commit -m "feat(commands): apply_review_result — guarded paste + clipboard restore"`

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

// [F11] PULL on mount: the backend emits "review-text" right after show(), which can race the listener
// registration (the emit may fire before this useEffect runs). So also pull the stored payload — whichever
// arrives first wins; setting the same text twice is harmless.
const pending = await commands.getPendingReview();
if (pending) { setText(pending[0]); setAiAvailable(pending[1]); setActiveAction("proofread"); }
```

**Step 2:** Chip row. `Proofread ·offline` always enabled; `Rewrite/Formal/Summarize` disabled when
`!aiAvailable` (with a tooltip/hint "Configure AI in Settings → Cleanup"). Selecting Proofread renders
`<ReviewPanel text={text} onResult={setReviewResult} />`. AI chips are wired for real in Phase B; in
Phase A they are visible-but-disabled (no local model yet, and if Ollama is configured they can call
`previewCleanup` as an interim — OPTIONAL, keep simple: leave disabled unless `aiAvailable`).
Put the gating in a pure helper `chipEnabled(action, aiAvailable)` (Proofread → always true; others →
`aiAvailable`) so the logic is trivially inspectable. **Honesty note:** the design promised a UNIT test
for this predicate, but the repo has no JS test runner (frontend "tests" = tsc/eslint only). Rather than
stand up vitest for one predicate, this is downgraded to **exercise-verified** (A11 confirms AI chips are
disabled when AI isn't configured). If a JS test harness is added later, add the true+false cases then.

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

**Step 2:** Use the `verify` skill / manual drive. Confirm, watching real behavior (each line is a
folded finding — this exercise is the ONLY proof for the OS-boundary ones):
- Select text in Notepad/browser, press `Ctrl+Shift+J` → card appears **at the cursor** (test on a
  **HiDPI / 150%-scale display and a second monitor** — `[F6]`), showing the Proofread result underlined.
- **[F2] Keyboard-first without a click:** with the card open but NOT clicked, press `Enter` → it Applies;
  press `Esc` (on a fresh open) → it closes. If the keys instead land in your document, force-foreground
  failed — investigate before shipping.
- Accept a fix, press `Apply` → text replaced in the source field; a **single `Ctrl+Z` reverts** it.
- **[F1] CLIPBOARD PRESERVED (critical):** put a known value on the clipboard (copy a password), do a full
  review→Apply, then paste (`Ctrl+V`) elsewhere → the **original value must still be there**. Repeat with
  review→`Esc` (cancel) → original clipboard still intact. This is the CRITICAL fold; verify it explicitly.
- **[F3] Wrong-window guard:** open the card, close the source app, press Apply → the result must NOT be
  pasted anywhere random; it should land on the clipboard with a warning logged (grep `could not refocus`).
- **[F9] Re-entrancy:** with the card open, press `Ctrl+Shift+J` again → no second card, source unchanged.
- **[F5] Kill-switch on the DEFAULT backend:** confirm `keyboard_implementation` is HandyKeys (the Windows
  default), toggle `selection_review_enabled` **off** → hotkey no longer fires. Toggle on → fires. Rebinding works.
- Grep the debug log for `Review-selection hotkey fired` and `apply_review_result: refocused=true` to
  confirm fire + refocus actually happened (silent-async-panic + focus gotchas).

**Step 3:** Run full test + lint gate: `cargo test --lib` (all pass incl. the **6** overlay_pos tests +
the defaults test), `tsc --noEmit`, `eslint`, `prettier --check`. Fix anything red.

**Step 4: [F15]** Finish Phase A per superpowers:finishing-a-development-branch. **Phase A is shippable
ONLY after F1 (clipboard preservation) is verified green in Step 2** — a v1 that eats the clipboard on
every use is not shippable. The AI chips are intentionally inert until Phase B; that's an acceptable v1.

---

# PHASE B — Local Gemma E2B backend (AI chips work fully offline)

> Phase B has genuine integration unknowns (a C++ inference crate on Windows, model download). It starts
> with a spike; the later task granularity is refined after B1 confirms the build. All AI actions route
> through the existing `post_process`/`resolve_cleanup` seam, so no Phase A overlay code changes.

## Task B1: Spike — add `llama-cpp-2`, confirm it builds & links on Windows

**Files:** `src-tauri/Cargo.toml`; a throwaway `examples/llama_smoke.rs`.

**Step 1: [F14]** Add the dep **optional + behind a default-OFF feature** so plain `cargo test`/CI never
builds the C++ inference stack. `cargo add` alone adds a NON-optional dep — do it by hand:
```toml
[dependencies]
llama-cpp-2 = { version = "*", optional = true }   # pin the real version

[features]
local-llm = ["dep:llama-cpp-2"]
```
Confirm it does not conflict with the existing GGML from `transcribe-cpp`.

**Step 2:** Write the smoke example **gated to the feature** so `cargo test` (which compiles examples)
doesn't drag it in:
```toml
[[example]]
name = "llama_smoke"
required-features = ["local-llm"]
```
The example loads a small GGUF and generates 10 tokens from a fixed prompt. Build it explicitly with
`cargo run --example llama_smoke --features local-llm`.

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
