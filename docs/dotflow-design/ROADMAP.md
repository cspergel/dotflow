# DotFlow — Roadmap

Working order agreed for the current arc. Check items off as they land; keep this file the single
source of truth for "what's next". Full context for each item lives in
[`SESSION-HANDOFF.md`](./SESSION-HANDOFF.md).

## Order

### 1. Typed text expander — backend (IN PROGRESS)

Type a dot-trigger (`.fu`) in **any** app → it's replaced by the saved phrase (the same library that
powers spoken triggers). Step 1 (safe foundation, off by default) is **done** (`8c4eb61`).

- [x] Step 1 — setting `experimental_typed_expander` (default OFF), self-injection suppression
      (`is_injecting()` / `InjectGuard`), pure tested core (`ExpanderBuffer`, `match_typed_trigger`).
- [x] Step 2/3 — Windows Raw Input keyboard monitor **implemented** (`dotflow/typed_expander/backend.rs`),
      builds + unit-tests green; **live smoke-test pending** (type `.fu` in another app):
  - `RegisterRawInputDevices` (keyboard, `RIDEV_INPUTSINK`) + message-only hidden window + `WM_INPUT`
    pumped on a **dedicated native thread**; chars decoded with `ToUnicodeEx` + a tracked 256-key state.
  - Feed printable chars → `push`; Backspace → `backspace`; Enter/Tab/Esc/arrows/nav/Delete → `reset`;
    modifiers/lock/function keys → ignore (don't wipe an in-progress trigger).
  - On `matched()`: if `!is_injecting()`, raise `injection_guard()` → erase the `.key` via
    `inject_field_edit(n, "")` → paste the expansion via `inject_bulk` → `consume(n)`; a ~300 ms trailing
    **settle** window swallows the async `WM_INPUT` echoes of our own keystrokes so it can't re-trigger.
  - Injection guard made **re-entrant** (counter) so the outer emit guard + inner paste guard nest safely.
  - Start/stop the monitor thread on the setting toggle (`change_typed_expander_setting` command) and at
    boot when already on; UI toggle in Advanced → Experimental with an explicit "monitors your typing" note.
  - Windows-only; a `#[cfg]` stub keeps other platforms compiling (mac/Linux backends can slot in later).
  - **Deviations from the original spec:** backspaces go through the tested `inject_field_edit` (enigo)
    rather than raw `SendInput`; self-suppression uses the guard + a time-based settle rather than HID-source
    filtering. **Known v1 limitation:** a trigger fires the instant it completes (`.fu` expands mid-word in
    `.fund`) — pick trigger keys that aren't word prefixes.

### 2. Housekeeping

- [ ] Disable the noisy Handy CI workflows (`build`, `main-build`, `release`, `nix-check`, `playwright`,
      `build-test`, `pr-test-build`); keep `code-quality` + `test`.
- [ ] Weekly upstream sync: `git fetch upstream && git log --oneline 0a59e1f..upstream/main`, cherry-pick
      worthwhile fixes, rebuild+test, update `FORK.md`. Last synced: `0a59e1f`.

### 3. Premium redesign (expanded window)

- [ ] Linear/Raycast look: surface-ladder colors + 1px hairline borders + **no drop shadows** +
      medium-weight (500) headings, tight tracking + constrained content width (~640–720px) + settings as
      grouped hairline-separated rows (not card-per-setting).

### 4. Clean-up-selected-text hotkey

- [ ] A 2nd hotkey that sends the SELECTED text to the post-process LLM (Ctrl+C → read clipboard → LLM
      cleanup prompt → paste result). Reuses existing post-processing + clipboard infra.

## Later / backlog

- Phone-as-microphone (LAN web page, QR pair, WebSocket audio → transcribe pipeline). Likely last.
