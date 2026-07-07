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
- [x] Step 2/3 — Windows Raw Input keyboard monitor **implemented + live-validated**
      (`dotflow/typed_expander/backend.rs`). Smoke-tested by driving synthetic keystrokes into a separate
      window: typing `.fix` erased the trigger and pasted "Fix the bug where " (clipboard preserved), and a
      lone sentence period was left literal. Caught + fixed a real bug along the way (empty-key phrases made a
      bare `.` trigger — see below).
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
  - **Bug fixed (found via smoke-test):** alias-only phrases are stored with `key = ""`, so `match_typed_trigger`
    built the trigger `".{key}" == "."` and expanded on **every lone period**. `PhraseTable::new` now drops
    empty keys (with a defensive guard in `match_typed_trigger`), fixing both the typed and spoken paths.
  - **Deviations from the original spec:** backspaces go through the tested `inject_field_edit` (enigo)
    rather than raw `SendInput`; self-suppression uses the guard + a time-based settle rather than HID-source
    filtering. **Known v1 limitation:** a trigger fires the instant it completes (`.fu` expands mid-word in
    `.fund`) — pick trigger keys that aren't word prefixes.

### 2. Housekeeping ✅

- [x] Disabled the noisy Handy CI workflows via `gh workflow disable` (not YAML edits, to avoid upstream-sync
      conflicts): `main-build`, `nix-check`, `playwright`, `build`, `build-test`, `pr-test-build`, `release`.
      Kept `code-quality` + `test`. Documented in `FORK.md` (§CI on the fork) with the re-enable command.
- [x] Upstream sync check (2026-07-07): `upstream/main` head is `0a59e1f` — the last sync point, so **no new
      commits to port**. Re-run `git fetch upstream && git log --oneline 0a59e1f..upstream/main` next week.

### 3. Premium redesign (expanded window) — good pause point (more polish possible later)

- [x] Surface-ladder look (off-white canvas → white panels → inset controls, green-biased hairlines, no
      shadows), Geist bundled font, emerald reserved for meaning, refined toggles/titlebar/inputs.
- [x] DotFlow-specific IA: grouped sidebar (DICTATE / REVIEW / SYSTEM) with Phrases elevated + emerald rail.
- [x] Trimmed model catalog (Parakeet V3/V2 · Whisper Small/Turbo/Large-v3 · Moonshine V2 streaming).
- [x] Super-compact "mini" bar tier; removed Handy donate + acknowledgements.
- [ ] _Optional future polish:_ deeper visual refinement (the design can still improve — paused here by
      choice to move to task 4). Remaining pages still on old tokens: History, Post-processing, Phrases,
      General, Debug (they work; not yet ladder-styled).

### 4. Clean-up-selected-text hotkey ✅

- [x] `cleanup_selection` hotkey (default **Ctrl+Shift+U** — NOT Ctrl+Alt+\*, which is AltGr on Windows and
      gets swallowed). Copies the selection → cleans it → pastes over it → restores the clipboard. One-shot
      `CleanupSelectionAction`; sync clipboard/keystroke work on a blocking thread, only the LLM awaited.
- [x] **Deterministic cleanup by default** (`dotflow/cleanup.rs`, 10 unit tests): whitespace, spacing around
      punctuation, sentence capitalization, "i"→"I". Conservative — preserves decimals/number groups/line
      breaks; leaves ambiguous grammar (its/it's) alone. Zero setup.
- [x] **Upgrades to the post-process LLM** when configured (local Ollama or cloud) via a fixed built-in
      cleanup prompt — reuses `post_process_transcription` with a settings clone, so the dictation path is
      untouched. Falls back to deterministic if the LLM isn't configured or fails.
- [x] **Footgun fix:** the shortcut validator now rejects modifier-less global bindings (a bare `l` was
      firing on every keypress). Escape + F-keys stay exempt. 5 unit tests. (`shortcut/tauri_impl.rs`)
- Live-validated: `hello  world ,its  me` → `Hello world, its me`.

### 5. Offline grammar cleanup + Grammarly-style review (IN PROGRESS)

Free, fully-offline grammar/spelling via **Harper** (`harper-core`, Apache-2.0) — no API key. The engine is
in and wired to the Ctrl+Shift+U hotkey (see commit `e7f8a75`).

- [x] Harper engine (`dotflow/grammar.rs`): `harper_cleanup` (auto-apply confident fixes) + `analyze`
      (return spans/kind/message/replacements for a review UI). Panic-safe. Cleanup tier order:
      post-process LLM → Harper → deterministic tidy. Unit-tested.
- [x] **Cleanup** settings section (hotkey, engine indicator, "Try it" box via `preview_cleanup`).
- [x] Hotkey reliability: wait for modifier release before the synthetic Ctrl+C, sentinel-based copy detect.
- [x] Moved the typed-expander + ding toggles out of Advanced→Experimental into the **Phrases** section.
- [ ] **Review panel** (NEXT): a Grammarly-style UI in DotFlow's own window — text with issues underlined,
      click-to-accept/reject each suggestion, then insert/paste. Data layer done (`analyze_text` command +
      `TextSuggestion`); build the panel UI (render text + highlight `start..end` spans + replacement
      chips + apply by char-splice). Feed it from: (a) **post-dictation** review, and (b) a **selection**
      hotkey (selected text or whole field).
- [ ] **Live trailing autofix** (later, experimental): correct completed words/sentences as you type in any
      app, reusing the typed-expander's Raw Input monitor + focus-change stop. No highlighting.
- [ ] **Dictionaries**: Medical/Legal wordlist toggles on top of Harper (`hunspell-en-med-glut`, OpenMedSpel).
- [ ] _Deferred:_ in-app highlighting _inside other apps_ (Gmail/Outlook) — needs per-app accessibility /
      overlay integration (the hard Grammarly path). Revisit only if we commit to it.

## Later / backlog

- Phone-as-microphone (LAN web page, QR pair, WebSocket audio → transcribe pipeline). Likely last.
