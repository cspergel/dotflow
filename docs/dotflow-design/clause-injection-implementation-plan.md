# DotFlow — clause-level continuous injection: implementation plan (Milestone 1, the "feel")

**Status:** the deterministic core is BUILT + TESTED (`src-tauri/src/dotflow/clause.rs`, `ClauseStream`, 26
tests green). The real-time glue below is DESIGNED but NOT built — it is a concurrency-sensitive change to the
recording pipeline that must be validated with a live mic (it cannot be tested headless), so it is written as a
plan to implement mic-in-hand, not blind.

## The goal (design §11b)
Handy buffers the whole utterance and injects once on release ("feels dead"). DotFlow injects **each VAD clause
as you speak**: silence gap → transcribe that clause with warm Parakeet → inject it immediately. Text accumulates
in the field a beat behind your voice = the Dragon feel.

## What's already in place
- **`ClauseStream`** (`dotflow/clause.rs`): the deterministic cadence — feed it a transcribed clause, it returns
  the exact chunk to inject (phrase-expanded + punctuated + correctly spaced vs the previous clause). Tested.
- **Injection primitive**: `PasteMethod::Direct` → `paste_direct` → `enigo.text()` (keystroke injection). Already
  the default (DotFlow flip). Injecting a chunk mid-dictation = call the same `enigo.text()` per chunk.
- **VAD**: `audio_toolkit/vad/smoothed.rs` is a speech/silence state machine (onset + silence counters); the
  recorder exposes `with_audio_callback`. Clause boundaries ARE detectable here.

## The change (hook points)
Today (`actions.rs` ~628–676): on stop, `rm.stop_recording()` returns the WHOLE buffer, then a single
`tm.transcribe(samples)` → one `paste`. Replace that, for the offline (non-streaming) path, with a live loop:

1. **Segment during recording.** In the recorder's audio callback, run the smoothed VAD. Track the current
   speech run; when the VAD transitions speech→silence for ≥ a redemption window (a clause boundary), close the
   segment: hand its samples to a transcription worker channel. (This mirrors the existing streaming worker's
   `StreamRouter::feed` + channel pattern — reuse that plumbing rather than invent new threading.)
2. **Transcribe per clause, off the audio thread.** A worker drains the segment channel, calls the warm Parakeet
   `transcribe(segment)` (~a few hundred ms at int8), and pushes the text into `ClauseStream::push`.
3. **Inject the returned chunk** on the main thread via the existing `enigo.text()` path (the `paste` chokepoint,
   minus the trailing-space logic which becomes per-utterance, not per-clause).
4. **On stop**, transcribe + inject any final open segment, then tear down (mirror `finalize_stream`).

## Concurrency + correctness (the part that needs care + a mic)
- Three threads already exist here (audio callback, transcription, main/UI). Injection MUST stay ordered — clause
  N before N+1 — so the worker is single-consumer and injects in receive order (a channel gives FIFO for free).
- **Barge-in / cancel**: honor `was_cancelled_since` between clauses (Handy already threads this).
- **Undo safety (design §11 "fast undo"): STILL OWED** — live injection makes a wrong clause land in the field;
  one-keystroke undo of the last injected chunk is a V1 requirement, not polish. Track injected chunk lengths.
- **VAD tuning is empirical**: the silence-redemption window that feels like "a clause" vs "chopped mid-sentence"
  can only be set by ear. This is the single biggest reason this step needs the mic, not just a compile.

## Status update — the LOGIC is now built + tested; only the cross-manager wiring remains
`src-tauri/src/dotflow/clause.rs` now contains the full clause loop, unit-tested end-to-end with fakes (31
dotflow tests green):
- `ClauseSegmenter` — audio frames + a VAD speech/noise bool → clause segments at silence gaps (tested with
  scripted verdicts; `silence_close_frames` is the redemption window to tune by ear).
- `ClauseStream` — a clause's text → the exact chunk to inject (phrase-expanded, punctuated, spaced).
- `ClauseInjectionLoop::push_frame(samples, is_speech, &mut transcribe, &mut inject)` and `.finish(...)` — the
  whole loop with `transcribe: FnMut(Vec<f32>) -> String` and `inject: FnMut(&str)` INJECTED, so it is fully
  tested without a model/VAD/enigo. `full_loop_injects_two_clauses_in_order_expanded_and_spaced` proves it.

**So the ONLY unverified work left is the real-time plumbing that provides those two closures + a VAD:**
1. A worker thread (mirror `StreamRouter`'s channel) fed raw frames from `create_audio_recorder`'s
   `with_audio_callback` when an `experimental_clause_injection` setting is on (default off — the batch path is
   untouched when off).
2. In the worker: a `SmoothedVad` (from the vad model path) turns each frame into `is_speech`; call
   `loop.push_frame(frame, is_speech, &mut transcribe, &mut inject)`.
3. `transcribe` = the warm `TranscriptionManager::transcribe(segment)`; `inject` = the `enigo.text()` path
   (`clipboard::paste` minus per-utterance trailing space). On stop, `loop.finish(...)` then tear down.

**Honest cross-manager caveat:** `create_audio_recorder` today has only `app_handle` + `stream_router` — NOT the
`TranscriptionManager`. So wiring `transcribe` means threading the transcription manager (or a transcribe channel)
into the recorder/worker, plus per-recording worker lifecycle. That is a real cross-manager change, not a local
edit — which is why it's staged as the mic-in-hand step rather than fabricated blind.

## Why it's not built blind
A real-time audio/threading change that compiles is not a working one; only dictating into it reveals whether
clauses segment at the right pauses, inject in order, and feel live rather than laggy or chopped. Per the design's
own rule ("be your own first user; the only test that matters for Milestone 1 is whether it feels like Dragon"),
this step is implemented with the mic in hand. The deterministic core it plugs into is done and proven.
