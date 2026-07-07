# DotFlow — Local Dictation Engine: Model & Stack Research

_Researched June 2026. Goal: the best fully-local, low-latency, streaming-capable ASR stack and the repos to steal from, with cross-platform (Windows-first) in mind. Your current reference point is Handy with Parakeet v3 + Canary 180m flash._

---

## TL;DR — the recommended stack

**Engine:** `sherpa-onnx` (k2-fsa) — one C/C++ runtime, ONNX-based, runs every model below, ships prebuilt wheels/binaries for Windows / macOS / Linux / Android / iOS, and has bindings in Python, Rust, C#, Node, Go, Swift, Flutter, etc. This is the layer that makes "local + cross-platform" actually tractable instead of a per-OS science project.

**Default model:** **Parakeet TDT 0.6B v3 (int8)** for the everyday dictation path. CC-BY-4.0 (commercial-OK), native punctuation + capitalization, 25 EU languages w/ auto-detect, and it runs fast on **CPU** in int8 — which is the whole reason Handy ships it as its CPU default. ~6.3% WER clean.

**Live/streaming path:** **Nemotron Speech Streaming 0.6B** (cache-aware streaming, ONNX) when you want true word-by-word partials. An April-2026 benchmark of 50+ configs across Whisper/Parakeet/Canary/Qwen3/Nemotron picked Nemotron streaming as the **best on-CPU real-time English** model. Parakeet via VAD+chunking gives you "near-live" without it.

**Cleanup LLM (optional, also local):** Qwen 2.5 1.5B via MLX (Mac) or a small model via Ollama/llama.cpp (Windows). But — see §6 — most "cleanup" should be deterministic, no model.

**Insertion (the Dragon feel):** **keystroke/text injection via `enigo`** (MIT, Rust, Win/mac/Linux) as the _primary_ path, clipboard-paste only as fallback. This is what makes text "pop in place" like Dragon instead of pasting with a seam. See §11.

**Feels-live cadence:** inject **per clause as you speak** (VAD-segmented), not one block at the end. This is the single biggest reason Handy feels dead and DotFlow won't. See §11.

**Punctuation toggle:** user chooses **auto-punctuation** (model adds it) vs **spoken-punctuation** ("period", "new line" — user dictates it), with an optional light-correction pass in spoken mode. See §12.

**Blueprint to clone:** **Handy** (`cjpais/Handy`) — Tauri + Rust + React, Whisper + Parakeet v3, Silero VAD, push-to-talk → paste, cross-platform. **Confirmed MIT-licensed — fully forkable for a commercial product.** It is almost exactly DotFlow's V1 engine. Start here. (Note: Handy itself uses `enigo` as its injection fallback — that's our primary.)

---

## 1. The models, ranked for this use case

### Parakeet TDT 0.6B v3 — the default pick

- **Architecture:** FastConformer encoder + Token-and-Duration Transducer (TDT) decoder, 600M params. The TDT decoder predicts tokens _and their durations_, so it skips silence and runs far faster than real-time.
- **Why it wins for dictation:** native punctuation + capitalization (no separate model), word-level timestamps, **CPU-friendly in int8** (Handy uses `parakeet-tdt-0.6b-v3-int8`), and a **CC-BY-4.0 license you can ship commercially**.
- **Accuracy:** ~6.34% WER clean, 11.66% at 0 dB SNR. Top-throughput multilingual model on HF Open ASR Leaderboard.
- **Languages:** 25 European, auto-detected. (v2 = English-only, slightly leaner if you only need EN.)
- **Caveat:** it's a _non-streaming_ model. You get "live" feel via VAD + chunking, not true token streaming. Great for push-to-talk-and-release; for word-by-word as-you-speak, see Nemotron.

### Canary 180m flash — the speed/lightweight option

- 182M params, encoder-decoder (FastConformer + Transformer decoder), **>1200 RTFx** — extremely fast, tiny footprint. CC-BY-4.0, commercial-OK.
- ASR + translation in EN/DE/FR/ES, optional punctuation/caps, experimental word timestamps.
- **Trade-off:** smaller and faster than Parakeet but generally a bit less accurate, and only 4 languages. Good when you want minimal RAM/disk or a fallback tier ("fast" mode). Worth A/B-ing against Parakeet on _your_ voice — you've seen it work well, which tracks.
- Bigger sibling **Canary 1B v2 / Canary-Qwen 2.5B** tops the leaderboard (~5.6% WER) but is heavier and the Qwen variant is really an ASR+LLM hybrid — overkill for dictation.

### Nemotron Speech Streaming 0.6B — the true-streaming pick

- **This is the answer to "live."** Cache-aware streaming ASR with punctuation, processes audio in small chunks (e.g. 560ms windows) and emits text incrementally with bounded latency.
- An April-2026 study ("Pushing the Limits of On-Device Streaming ASR," arXiv 2604.14493) benchmarked 50+ configs across Whisper, Parakeet TDT, Canary, Conformer-Transducer, Qwen3-ASR, and Nemotron — and selected **Nemotron streaming as the strongest real-time English model on resource-constrained CPU**, after re-implementing it in ONNX Runtime with quantization. They explicitly trade off algorithmic delay vs effective latency.
- Variants: English-only 0.6B (verbatim, keeps disfluencies — good when every word matters) and a multilingual 3.5 0.6B.
- Available through `parakeet-rs` and sherpa-onnx.

### Parakeet Flash — the "feels INSTANT" streaming option (validated in production)

- **This is the streaming model to reach for first, ahead of Nemotron.** FluidVoice (popular Mac dictation app, 5.8k★) ships **Parakeet Flash**: an English-only local **streaming** model for low-latency word-by-word live transcription. Their own description: _faster than Parakeet TDT but less accurate — feels INSTANT._
- It confirms the exact tradeoff we predicted: true word-by-word streaming costs accuracy vs. Parakeet TDT, but delivers the live feel. English-only, which suits the **coder launch audience** perfectly.
- FluidVoice got dictation latency down to **<~100ms** with this family on Apple Silicon — an existence proof that "feels like Dragon" is achievable with Parakeet-class models. (That's Neural Engine / CoreML; your Windows-CPU-via-sherpa-onnx numbers will be slower — the spike measures your real hardware.)
- **Decision:** if clause-cadence on Parakeet TDT (the cheap path) isn't live enough, try **Parakeet Flash before Nemotron** — it's English-only and instant, ideal for the coder wedge. Nemotron remains the multilingual / CPU-benchmark-winning alternative.

### Cohere Transcribe — the punctuation/numbers accuracy candidate

- FluidVoice added Cohere Transcribe and calls it _very accurate with punctuation and numbers_ — directly relevant to the medical-accuracy problem (drug doses, numbers, structured A/P).
- Available via `parakeet-rs` (which bundles Parakeet TDT + Nemotron + Cohere) and runnable locally with Neural Engine/GPU split execution.
- **Worth A/B-ing** against Parakeet + RxNorm correction specifically on numbers and dosages, where it may reduce the correction burden. Add to the spike's model comparison.

### Whisper (large-v3 / turbo / faster-whisper) — the known quantity

- Still the accuracy baseline and what most wrappers ship. **But:** slower on CPU, no native streaming (chunk-based only), and the large models want a GPU to feel snappy.
- Keep it as an **optional engine** (some users trust it, fine-tuned medical Whisper models exist) but it should **not** be your low-latency local default. Parakeet beats it on CPU latency handily — one Rust author measured Parakeet on CPU _faster_ than Whisper on Metal on an M3.

### Practical ranking for DotFlow

1. **Parakeet TDT v3 int8** — default, push-to-talk, CPU, commercial license. ✅
2. **Parakeet Flash** — the "feels instant" English-only streaming live-mode (try before Nemotron). ✅
3. **Nemotron streaming 0.6B** — multilingual / CPU-benchmark-winning streaming alternative. ✅
4. **Cohere Transcribe** — punctuation/numbers-accuracy candidate (test for medical). ✅
5. **Canary 180m flash** — "fast/light" tier + fallback. ✅
6. **Whisper (faster-whisper / WhisperKit)** — optional engine for trust/accuracy/fine-tunes.

---

## 2. "Live" — what it actually means here (important nuance)

There are two different things people call "live," and conflating them is the #1 architecture mistake:

**A. Live preview (near-live).** VAD detects speech, you chunk the audio, and a _non-streaming_ model (Parakeet, Canary, Whisper) transcribes each chunk fast enough that text appears a beat behind you. This is what Handy, Speak2, and most apps actually do. It's simpler, more accurate, and good enough for dictation where you mostly push-to-talk and release. The `sherpa-onnx-vad-microphone-simulated-streaming-asr` example does exactly this with Silero VAD + Parakeet int8.

**B. True streaming.** A cache-aware streaming model (Nemotron) emits tokens word-by-word with bounded latency _while you're still talking_. Needed only if you want real-time captions / talk-and-watch-it-type. Costs accuracy and complexity.

**Recommendation:** ship **A (VAD + Parakeet chunking)** for V1 — it's the proven, accurate, lower-effort path and matches the push-to-talk UX. Add **B (Nemotron streaming)** as a "live mode" toggle later for the demo-wow and for users who dictate in long continuous flows. This mirrors what the benchmark paper and the real apps do.

Silero VAD is the standard gate in front of either path — it's what stops you from transcribing silence and is in every serious local stack.

---

## 3. The engine decision: sherpa-onnx vs NeMo vs MLX vs faster-whisper

| Engine                 | Runs                                                                           | Platforms                                                | Streaming                       | Verdict                                                                 |
| ---------------------- | ------------------------------------------------------------------------------ | -------------------------------------------------------- | ------------------------------- | ----------------------------------------------------------------------- |
| **sherpa-onnx**        | Parakeet, Nemotron, Canary, Whisper, Paraformer, SenseVoice, + VAD/punctuation | Win/mac/Linux/Android/iOS/embedded, 12 language bindings | ✅ online + simulated-streaming | **Use this.** One runtime, every model, every OS, prebuilt binaries.    |
| **NVIDIA NeMo**        | everything, reference impl                                                     | Python, GPU-centric                                      | ✅                              | Great for research/fine-tuning, heavy for shipping a desktop app.       |
| **parakeet-mlx / MLX** | Parakeet, Qwen refiners                                                        | **Apple Silicon only**                                   | ✅ (160ms chunks)               | Best _Mac_ perf via Neural Engine, but not cross-platform.              |
| **faster-whisper**     | Whisper family only                                                            | Win/mac/Linux (CTranslate2)                              | chunked only                    | Fine for a Whisper-only fallback; not your main engine.                 |
| **parakeet-rs**        | Parakeet TDT + Nemotron streaming + Cohere                                     | Rust, ONNX Runtime, CPU/WebGPU                           | ✅ true streaming API           | Excellent if you go Rust/Tauri; clean streaming API (160/560ms chunks). |

**The call:** `sherpa-onnx` as the universal engine. If you commit to Tauri+Rust (Handy's path), `parakeet-rs` is a strong alternative/complement with a genuinely nice streaming API. On Mac specifically, MLX/FluidAudio (Neural Engine) is faster — worth a Mac-only fast path later, but don't start there.

---

## 4. Repos to steal from — the real landscape

### Clone-the-architecture tier

- **`cjpais/Handy`** — _your reference, and the blueprint._ Tauri (Rust + React/TS), cross-platform Win/mac/Linux, Whisper + **Parakeet v3 int8 CPU**, Silero VAD, configurable hotkey → record → paste. Auto-discovers custom Whisper GGML models. This is ~80% of DotFlow's V1 engine already built. **Start by reading this end to end.**
- **`egsok/openwhispr-custom`** — Electron + React, **local (Parakeet via sherpa-onnx / Whisper via whisper.cpp) AND cloud BYOK** in one app, custom dictionary with **auto-learn from your corrections**, multi-provider LLM cleanup (OpenAI/Anthropic/Gemini/Groq/local llama.cpp), native helpers for fast paste on Win/mac/Linux. This is the closest thing to your "provider router + BYO + local" vision already wired. **Steal the dictionary auto-learn and the paste helpers.**

### Mac-specific, great UX patterns

- **`altic-dev/FluidVoice`** — _the further-along, GPL'd cousin — design reference & competitive benchmark, NOT a code source._ Popular Mac dictation app (**5.8k★, 766 commits, 35 releases, v1.6.1 as of June 2026** — ~doubled since our earlier note), **99.7% Swift / macOS-native** — so there is literally nothing here to fork for cross-platform; it's a feature benchmark only, Handy stays your Rust/Tauri fork base. **License trap: GPLv3 since 2026-02-23; Apache-2.0 before that.** Patterns only. What it proves/teaches: multi-engine router (Nemotron 3.5, **Parakeet Flash**, Parakeet TDT v3/v2, **Cohere Transcribe**, Apple Speech, Whisper) with latency-aware routing; **Live Preview** — real-time transcription overlay with notch support (words appear as you speak **in FluidVoice's own overlay**, not streamed into the field — final text injected via accessibility APIs on completion; this is the key architecture — see §11b); **Write/Rewrite modes** (dictate inline OR select-text-and-rewrite by voice in any field — _shipping_); **Command Mode** (control the Mac by voice: launch apps, run shortcuts, system actions — a _shipping production version_ of the Voice-Addressable-Targets/OpenClaw offshoot ideas); v1.6.0 rebuilt Parakeet to **near-zero delay** between speaking and seeing words; **Reliable Paste** insertion mode (confirms injection needs a paste fallback for browsers — validates §11a); **Per-App Configuration** (prompt sets per app — lighter saved-workspaces); **Fluid Intelligence** = a _separately maintained, closed-source_ local enhancement model (open-core monetization template: free open app, paid closed model). **Its gaps = DotFlow's space: it's Mac-only (Windows/iOS only on a waitlist), has NO phrase/dot-phrase library, and NO domain vocab packs. Win on cross-platform + phrases + vocab, not on out-Mac-ing it.**
- **`zachswift615/speak2`** — Local mac dictation, WhisperKit + Parakeet (FluidAudio), **live transcription overlay**, push-to-talk OR toggle, **personal dictionary with phonetic matching** (alias "Cooper Netties" → "Kubernetes"), built-in MLX refiner (Qwen 2.5 1.5B) OR Ollama. Excellent reference for the dictionary/alias UX and the live overlay.
- **`moona3k/macparakeet`** — Parakeet TDT on Apple Neural Engine via FluidAudio CoreML. Notable: a **sub-1ms deterministic vocabulary/cleanup pipeline** ("aye pee eye" → "API", capitalize "Kubernetes"), Voice Return trigger phrases, Transforms (select text + hotkey → LLM rewrite). **Steal the deterministic-cleanup design** — it's exactly your "raw by default, no LLM needed for most cleanup" principle, proven.
- **`osadalakmal/parakeet-dictation`** — minimal Parakeet (MLX) push-to-talk + voice-driven select-and-rewrite. Good small reference.

### Engine / streaming primitives

- **`k2-fsa/sherpa-onnx`** — the engine. Has copy-pasteable examples: `parakeet-tdt-simulate-streaming-microphone-cxx-api.cc`, `sherpa-onnx-vad-microphone-simulated-streaming-asr` (VAD+Parakeet+mic), Node/Electron JS API, Go/C#/Swift bindings. The `homophone replacer` feature is a built-in term-correction primitive.
- **`altunenes/parakeet-rs`** — Rust, ONNX Runtime, **true streaming** (Parakeet TDT + Nemotron cache-aware + Cohere), CPU or WebGPU (Metal on Mac). Clean API: feed 160ms (Parakeet EOU) / 560ms (Nemotron) chunks, get text. If you go Rust, this is the streaming core.

### Don't-start-here (reference only)

- NVIDIA **NeMo** — for fine-tuning a medical Parakeet later, not for shipping.
- Meeting-recorder forks — scope creep; you decided against meeting-first.

---

## 5. Cross-platform reality (Windows-first)

- **Windows CPU is the gating case.** Parakeet v3 int8 via sherpa-onnx is _designed_ for this and is Handy's CPU default — so this is a solved, shipping configuration, not a gamble. sherpa-onnx ships Windows binaries and a `sherpa_onnx_windows` package; the Go/C#/Node bindings all have real-time-mic-from-Windows examples.
- **GPU is a bonus, not a requirement.** With an NVIDIA GPU you can run larger/faster, but the int8 CPU path means a clinician on a normal laptop still gets usable dictation.
- **Mac gets a faster path later** via MLX/FluidAudio (Neural Engine) — but the same sherpa-onnx code runs there too, so you're never blocked.
- **The latency number to verify on YOUR hardware:** sherpa-onnx reports RTF ~0.3 for Parakeet int8 on CPU (2 threads) on their test wav — i.e. ~3x faster than real-time. That implies a short utterance transcribes in a few hundred ms after release, which is in "feels instant" territory. **But measure it on the actual CPUs your users have** — that's what the spike's latency timer is for.

---

## 6. Cleanup: mostly NOT an LLM (this saves you cost AND latency)

The macparakeet and speak2 designs validate your earlier instinct: **most "cleanup" is deterministic and needs zero model.**

- macparakeet runs its whole vocabulary/cleanup pipeline in **under 1ms**, no AI: custom word replacement, capitalization rules, voice-command stripping ("press return" → simulated Return).
- speak2 does phonetic-alias dictionary matching ("Cooper Netties" → "Kubernetes") deterministically.
- This is also your RxNorm/fuzzy-correction layer from the spike — same idea, applied to drug names.

**So:** deterministic pass handles 70%+ (punctuation, capitalization, dot-phrase expansion, custom dictionary, homophone/term correction). Reserve a **local LLM** (Qwen 2.5 1.5B via MLX/Ollama) only for explicit "rewrite this / make professional / format as A/P" transforms, with cloud BYO-key as an optional quality upgrade. sherpa-onnx even ships a `homophone replacer` you can use as the correction primitive.

---

## 7. Recommended DotFlow engine architecture

```
mic ─► Silero VAD ─► [engine router] ─► raw text ─► deterministic cleanup ─► (optional LLM) ─► INJECT (per clause)
       (clause-           │                              │
        level             ├─ Parakeet TDT v3 int8        ├─ custom dictionary (RxNorm / code / aliases)
        segments)         │   (default)                  ├─ PUNCTUATION TOGGLE: auto | spoken | raw
                          ├─ Nemotron streaming 0.6B     ├─ dot-phrase expansion (parse BEFORE inject)
                          │   (optional live mode)       └─ homophone/fuzzy term correction
                          ├─ Canary 180m flash (fast)
                          └─ Whisper (optional)          INJECT = enigo.text() primary, clipboard fallback
                                                         + live overlay (waveform + in-flight partial)

  all via sherpa-onnx · one runtime · Win/mac/Linux/iOS/Android · inject continuously = feels live
```

Provider router is engine-agnostic by design (OpenLess/openwhispr already prove this), so local vs cloud and Parakeet vs Nemotron vs Whisper are just config — exactly the interchangeability your v2 plan calls for.

---

## 8. What I'd actually do (build sequence)

1. **Fork Handy (MIT).** It's your engine — cross-platform, Parakeet v3 int8 + Silero VAD + enigo injection already wired. Confirm latency on your Windows box and Mac.
2. **Flip insertion to injection-first** (§11a): `enigo.text()` primary, clipboard demoted to fallback for huge blocks. Test the Dragon feel across Cursor/Gmail/web-EMR/Word.
3. **Add clause-level continuous injection + live overlay** (§11b) — this is what makes it feel live, and it's V1 core, not a later nicety. Warm-load Parakeet so there's no cold lag.
4. **Steal the deterministic cleanup + dictionary** patterns (macparakeet/speak2/openwhispr) — reimplement in your own code, wire in RxNorm/term-correction. Add the **punctuation toggle** (§12) here — it rides the same pipeline.
5. **Layer the phrase engine + parse-before-inject** (§11c) — your wedge, your original code.
6. **Add Nemotron true-streaming as an optional "live mode"** _only if_ clause-cadence isn't smooth enough (verify its license first).
7. **Canary 180m as the "fast/light" tier**, Whisper as the optional trust/fine-tune engine.
8. **Mac fast path (MLX/FluidAudio)** later; same sherpa-onnx fallback everywhere else.

---

## 9. Licensing — verified verdicts (what you can take vs redo)

I checked each load-bearing piece. The headline: **the entire core stack is MIT / Apache / CC-BY — genuinely shippable in a paid, closed-source product.** The traps are all in the _adjacent_ category (GPL/AGPL dictation apps), so the rule is "know which repos to avoid copying code from," not "this whole space is poisoned."

### Take freely (permissive — copy code, ship closed-source)

| Component                                | License           | Verdict                                                                                                                                                                                                                                             |
| ---------------------------------------- | ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Handy** (`cjpais/Handy`)               | **MIT** ✅        | Clone, fork, lift code. Confirmed MIT (CONTRIBUTING + README). This is your blueprint.                                                                                                                                                              |
| **OpenWhispr** (`OpenWhispr/openwhispr`) | **MIT** ✅        | The upstream is MIT — "free for personal and commercial use." Steal the provider-router + BYOK + auto-learn-dictionary patterns. (Note: the `egsok/openwhispr-custom` fork I cited earlier inherits MIT, but always re-check a fork's own LICENSE.) |
| **enigo** (input injection)              | **MIT** ✅        | Our primary insertion layer. Native `enigo.text("…")` Unicode injection + `enigo.key()` keystrokes on Win/mac/Linux. ~62k downloads/mo, mature.                                                                                                     |
| **sherpa-onnx**                          | **Apache-2.0** ✅ | The engine. Ship it.                                                                                                                                                                                                                                |
| **Silero VAD**                           | **MIT** ✅        | The clause-segmentation gate. Ship it.                                                                                                                                                                                                              |
| **whisper.cpp / ggml**                   | **MIT** ✅        | If you offer a Whisper fallback engine.                                                                                                                                                                                                             |
| **Whisper weights**                      | **MIT** ✅        | Same.                                                                                                                                                                                                                                               |

### Take with attribution only (CC-BY-4.0 — ship, just credit NVIDIA)

| Component                     | License                                                                                                                                                | Verdict                                    |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------ |
| **Parakeet TDT v3 / v2**      | **CC-BY-4.0** ✅                                                                                                                                       | Default model. Commercial OK, attribute.   |
| **Canary 180m flash / 1B v2** | **CC-BY-4.0** ✅                                                                                                                                       | Fast/light tier. Commercial OK, attribute. |
| **Nemotron streaming 0.6B**   | check the specific HF card (NVIDIA models are usually CC-BY-4.0) — **verify before shipping the live mode.**                                           |
| **Parakeet Flash**            | verify the model card — likely CC-BY-4.0 like the Parakeet family, but confirm before shipping the streaming live mode.                                |
| **Cohere Transcribe**         | check Cohere's model license/terms — may differ from the NVIDIA models; **verify before bundling** (it could carry usage terms even when run locally). |

### Patterns-only — do NOT copy code (copyleft traps in this category)

The dictation space is full of GPL/AGPL apps. Read them for ideas, never paste their code into DotFlow:
| Repo | License | Why avoid copying |
|---|---|---|
| **FluidVoice** (`altic-dev`) | **GPL-3.0** (since 2026-02-23; Apache-2.0 before) ⚠️ | The most advanced reference — Write/Rewrite modes, Parakeet Flash, Cohere, Reliable Paste, Command Mode, per-app config. **Patterns & benchmark only.** A pre-Feb-23 Apache fork is possible but forfeits everything shipped since. |
| **VoiceInk** | GPL-3.0 ⚠️ | Mac-native, nice UX — concepts only. |
| **nerd-dictation** | GPL-3.0 ⚠️ | Good typing-backend patterns — concepts only. |
| **savbell/whisper-writer** | GPL-3.0 ⚠️ | Concepts only. |
| **VoiceTypr** | **AGPL-3.0** 🚫 | Strongest copyleft; network-use triggers source disclosure. Hard avoid. |
| **Espanso / hallelujahIM** | GPL-3.0 ⚠️ | Expander concepts only (as already noted). |
| **Whispo** | AGPL-3.0 🚫 | Concepts only. |

### macparakeet / speak2 — verify individually

These have great deterministic-cleanup and live-overlay patterns. **Check each repo's LICENSE before lifting code** — if MIT/Apache, take freely; if GPL, reimplement the pattern in your own code. The _ideas_ (sub-1ms deterministic cleanup, phonetic-alias dictionary, live overlay) are not copyrightable — only their specific code is. You're reimplementing these anyway to wire in RxNorm.

**Bottom line:** Handy (MIT) + OpenWhispr (MIT) + enigo (MIT) + sherpa-onnx (Apache) + Silero (MIT) + Parakeet/Canary (CC-BY) is a **100% commercially-shippable foundation with zero copyleft.** You can fork Handy directly as the starting point and never touch a GPL line. The phrase engine, RxNorm layer, punctuation toggle, and live-injection cadence are your own original code on top.

---

## 10. Open questions to resolve in the spike

- **Parakeet v3 int8 latency on a low-end Windows clinician laptop** — feels-instant or 2–3s? Determines whether CPU-only is the floor or you nudge some users to GPU.
- **Parakeet vs Canary 180m on your voice + medical terms** — you've liked both; A/B them with the RxNorm correction pass on and off.
- **enigo injection feel across your real apps** — does `enigo.text()` land instantly and correctly in Cursor (Electron), Gmail/web-EMR (contenteditable), and Word? This is the Dragon-feel make-or-break. Clipboard only where injection fails.
- **Clause-level injection cadence** — does injecting per VAD segment actually _feel_ live, or is Nemotron true-streaming needed?
- **Nemotron streaming worth the complexity?** Only if clause-cadence isn't live enough. Verify its license first.
- **Deterministic cleanup coverage** — how much of "cleanup" you can do with zero LLM (likely most of it).

---

## 11. Insertion + "feels live" architecture (the Dragon feel)

This is the part that separates DotFlow from a Whisper wrapper. Two distinct mechanisms, both required.

### 11a. Inject, don't paste

Dragon's text "pops into place" because it **injects text directly** (keystrokes / text-services API), never clipboard-paste. Paste has a permanent seam: clipboard clobber, flicker, lands wrong in some fields. Three methods, priority order:

1. **Text/keystroke injection (`enigo.text()` / `enigo.key()`)** — PRIMARY. No clipboard touched, lands in the field's own formatting, works in paste-blocked fields, looks exactly like Dragon dropping text in. MIT, cross-platform. For dot-phrases (a few hundred chars) it's effectively instant.
2. **Accessibility/text-services API** (Win UI Automation TSF, mac AXUIElement) — the _purest_ path in native fields, but weak in web/Electron and more per-OS work. Optional later upgrade.
3. **Clipboard paste + restore** — FALLBACK ONLY, for very long blocks (where "typing" injection becomes visibly slow) or fields where injection fails.

**This reverses the original spike's ordering** (which tested clipboard first). For the Dragon feel, injection is primary and clipboard is the demoted fallback. The injector should pick method by payload size: keystroke-inject phrases & dictation, clipboard-paste only huge multi-paragraph templates.

**Real-world validation:** FluidVoice — a mature Mac app using accessibility-API text injection — _still_ had to ship a **"Reliable Paste" insertion-mode setting** for dependable insertion across browsers and desktop apps. That confirms two things: (1) injection-primary is right, and (2) you _will_ need a paste fallback specifically for browser/Electron fields where injection is unreliable. So the fallback isn't a nice-to-have — it's a required setting, and web fields are exactly where it earns its place (reinforcing why the browser extension matters).

### 11b. Inject per clause, not at the end (the live feeling)

Handy feels dead because it **buffers the whole utterance and injects once on release.** Even with non-streaming Parakeet, you get a live feel by injecting continuously:

- **Silero VAD** closes a segment at each natural micro-pause (~1–2s of speech / each clause).
- That segment transcribes immediately with **warm** Parakeet (int8, ~3× real-time → a few hundred ms).
- It **injects right then.** As you talk: clause appears, you keep talking, next clause appears a beat later. To your eye, text accumulates _while you speak_ — the Dragon feel — with the model you already have.
- A **floating overlay** shows the in-flight partial + a waveform so there's never a dead "is it listening?" moment (speak2 does exactly this).

**Key architecture confirmation from FluidVoice (5.8k★, shipping):** its "Live Preview" shows words appearing as you speak **in its own notch overlay — NOT streamed character-by-character into the destination field.** The final text is injected into the app once you finish. This is the important, easier target: **you don't need to stream into the actual text field to feel live — you need (1) a live preview overlay + (2) fast final injection.** The overlay can use any model (even a fast/rough one) for the preview while the accurate model produces the final insert. This is cheaper and more robust than trying to inject partials into arbitrary fields, and it's what a popular production app proves users read as "live."

### 11c. Parse-and-expand BEFORE inject

The dot-phrase magic: when a segment is a command ("insert COPD plan"), the parser expands it to the full template **before** injection, so the finished text lands as one clean block — you never see "insert COPD plan" appear then get replaced. Free dictation injects immediately; commands wait a beat to resolve.

**Command-buffer nuance:** if a trigger spans a pause ("insert COPD…[pause]…plan"), hold injection when a segment _starts_ with a trigger word (`insert`, `expand`, a `.`-cue) until the phrase resolves, then drop the expansion as one block. Dragon does this too — commands feel slightly more deliberate than dictation, which is fine.

### 11d. Where true streaming fits — Parakeet Flash first, then Nemotron

For the **dot-phrase/command** case, streaming is irrelevant — the phrase is canned, it lands as one instant block. Streaming only matters for **long free-form dictation** where clause-cadence might still feel chunky. So: ship 11a–11c first (clause injection feels live on Parakeet TDT). If that's not live enough, add a true-streaming "live mode" — and try **Parakeet Flash before Nemotron**: it's English-only, "feels INSTANT," and FluidVoice proves it works in production for exactly this. Nemotron is the multilingual / CPU-benchmark-winning alternative. Validate in the spike before building the streaming pipeline either way.

**This moves continuous-injection from "phase 5 nicety" to V1 core** — it's the thing that makes DotFlow feel like Dragon.

---

## 12. Punctuation: auto vs spoken (user toggle)

Different users want opposite things, and forcing one breaks the feel. Ship a **toggle**:

### Mode A — Auto-punctuation (default)

The model inserts punctuation/capitalization. Parakeet and Canary both emit **native punctuation + capitalization**, so this is free — no extra model. Best for casual users and prose dictation.

### Mode B — Spoken punctuation (Dragon-style, power users)

The user dictates punctuation explicitly: "period", "comma", "new paragraph", "new line", "open paren". The engine:

1. **Strips the model's auto-punctuation** (or runs a no-punctuation decode where supported), so the model doesn't double up.
2. **Maps spoken tokens → marks** via a deterministic command table ("period"→".", "new line"→`\n`, "cap that"→capitalize prior word). This is the same <1ms deterministic layer as cleanup (macparakeet's "Voice Return" trigger is exactly this pattern).
3. **Optional light correction:** if the user _mixes_ modes (says "period" but the model also guessed one, or forgets a cap), an optional pass fixes obvious conflicts — double periods, missing capital after a spoken period — **without** rewriting content. This is rule-based, not LLM, and stays off by default for verbatim purists.

### Why this matters

- Clinicians and lawyers trained on Dragon expect spoken punctuation and find auto-punctuation _wrong_ (it guesses sentence breaks they didn't intend).
- Coders dictating into an IDE often want neither — raw tokens, no auto-caps mangling `camelCase`.
- So the toggle is really **three states**: auto / spoken / raw (no punctuation processing at all).

### Implementation note

This rides on the deterministic cleanup pipeline (§6) — it's a command table + a strip/keep flag on the model's punctuation output, not a separate subsystem. The correction-if-mixed option is a few conflict rules, off by default. Cheap to build, high-value for the Dragon-trained audience.
