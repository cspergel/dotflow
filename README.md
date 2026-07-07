# DotFlow

**Voice dictation that triggers your reusable language — speak, expand, and it lands live in any field.**

DotFlow is a local‑first, privacy‑focused desktop dictation app. Hold a shortcut, talk, and your words appear
**as you speak** in whatever field you're already using — plus your own **dot‑phrase / voice‑alias macros** so
a spoken trigger drops in a saved block of text. Everything runs on your machine; nothing goes to the cloud.

> DotFlow is a fork of **[Handy](https://github.com/cjpais/Handy)** (MIT, © CJ Pais). It rides Handy's excellent
> local speech‑to‑text engine (Parakeet via `transcribe-rs`, Silero VAD, Tauri shell) and adds the DotFlow
> product layer on top. See [`FORK.md`](./FORK.md) for how we track upstream.

## What makes it DotFlow

- **Live in‑field injection** — text streams into the focused field _as you speak_ (the "Dragon feel"), not
  pasted once at the end. Tunable to your machine (per‑character timing + write throttle) so it stays clean.
- **Dot‑phrase / voice‑alias macros** — say `insert follow up` (or set a `.fu` trigger) and your saved text
  block drops in as one clean block. Matching is case‑, punctuation‑, and hyphen‑insensitive, so real ASR
  output ("Insert follow‑up.") still fires the trigger.
- **Editable phrase library** — a simple in‑app **Phrases** page (trigger → text). Edits apply on your very
  next dictation; no restart, no config files.
- **Dragon‑style UI** — a frameless app with a compact, always‑on‑top **status bar** you can expand to the
  full window. Mic shows amber on standby, green while dictating.
- **Local & private** — Parakeet / Whisper models run offline on your CPU; your voice never leaves the machine.

## How it works

1. **Press** (or hold) your shortcut to start recording.
2. **Speak** — committed words stream into the focused field live; spoken macro triggers are buffered and
   expanded as a block.
3. **Release** — the final text is synced and any resolved macro is inserted.

Silence is filtered with Silero VAD; transcription uses your choice of local model (Parakeet V3 by default).
Runs on Windows, macOS, and Linux (developed and tuned primarily on Windows).

## Build from source

Prerequisites: Rust (stable, MSVC on Windows), Node + [Bun](https://bun.sh), and the
[Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform.

```sh
bun install
# dev (Windows tip: a short CARGO_TARGET_DIR avoids the 260‑char path limit)
CARGO_TARGET_DIR=C:/dtfb bun run tauri dev
# release build
bun run tauri build
```

Run the DotFlow unit tests (the phrase engine + field‑streamer are pure and tested):

```sh
cd src-tauri && cargo test --lib dotflow
```

## Usage

- Set your dictation shortcut in **General**, pick a model in **Models** (Parakeet V3 is a good default;
  streaming models like Parakeet Unified give the most live feel).
- Add your own inserts in **Phrases**: a spoken trigger (e.g. `insert follow up`) and/or a typed dot key
  (e.g. `fu`), plus the text it inserts.
- Turn on live field streaming in **Advanced → Experimental** if you want text to appear as you speak; the
  per‑character delay + throttle sliders there let you dial it in if your machine drops/repeats keys.

## Credits & license

DotFlow builds on the work of **[Handy](https://github.com/cjpais/Handy)** by CJ Pais and contributors, and on
`transcribe.cpp` / `ggml` (Georgi Gerganov and contributors), Silero VAD, and the Tauri ecosystem. DotFlow is
released under the **MIT License** (see [`LICENSE`](./LICENSE)); Handy's original copyright is retained.
