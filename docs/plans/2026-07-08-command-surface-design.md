# P1 — Command surface for the review overlay (design draft)

> Status: **draft for review — NOT approved to build.** Implementation is gated on the prior work
> (selection overlay + local AI + Nemotron, branch `feat/selection-review-overlay`) being **live-tested and
> merged** first. This doc is a proposal to react to, with open questions flagged.
> Context: [`ROADMAP.md` §AI actions](../dotflow-design/ROADMAP.md).

## Goal

Turn the selection review card into a single, non-bloating **command surface** that scales to any number of
AI actions without a growing chip toolbar. The card already exists (proofread + Rewrite/Formal/Summarize
chips); this evolves it so new capabilities (Translate, Expand, Extract, Reply, custom instructions, …) are
**prompts behind one input**, not new buttons.

## The core idea

One card, three zones:

```
┌─────────────────────────────────────────┐
│ Review                            ⠿ drag │
├─────────────────────────────────────────┤
│ [ Proofread ]  [ Rewrite ]  [ Translate ]│  ← 2–3 PINNED quick actions (user-configurable later)
│                                           │
│  › Type or say what to do…          [→]   │  ← THE COMMAND INPUT (type OR dictate)
│    e.g. "make this a bullet list",        │
│         "reply saying I'll be late"       │
│                                           │
│  Suggested: Reply · Formal · Summarize    │  ← CONTEXT-AWARE chips (optional; see Q3)
├─────────────────────────────────────────┤
│  <result / proofread panel / before→after>│
├─────────────────────────────────────────┤
│              Close   Copy   Apply         │
└─────────────────────────────────────────┘
```

- **Command input** is the star: free-form intent, typed or **dictated** (reuses the STT pipeline). It is
  simultaneously the custom-instruction box (P1), the entry point for every named action (P2), and the seed
  of dictation command mode (P3).
- **Pinned quick actions**: the 2–3 most-used (default: Proofread, Rewrite, Translate). Later user-configurable.
- **Context suggestions** (optional, Q3): cheap heuristics surface likely actions for *this* selection.

## Routing: how free-form intent becomes a transform

Two options — **recommend starting with A, keep B as an upgrade:**

- **A — model-direct (simplest):** wrap the user's instruction as the system prompt and run it straight
  through `ai_transform` (new action `"custom"` carrying the instruction). One new code path; the model
  interprets "translate to Spanish" etc. Cheapest, and Gemma-class models handle it well.
- **B — tiny intent classifier (later):** a 0.5–1.5B model (or rules) maps the instruction to a *named*
  action + args ("translate" + lang="es") so we can apply a tuned prompt and show the right result view. More
  reliable for the named actions and needed for P3's hands-free mode, but more moving parts. Layer it in when
  P3 lands; A is enough for P1.

## How P2 named actions slot in (no bloat)

Each named action (Translate, Tone, Expand, Extract, Reply, SOAP) is just:
1. a system prompt (backend `system_prompt_for` already exists — extend it), and
2. an entry in a **"more actions" list** (a compact popover from a `+`/`⌘K` affordance), with the top few
   promotable to pins.

So adding an action = a prompt + a list entry, never a new always-visible button. Translate needs a target
language — resolve via the command input ("translate to French") or a tiny inline language picker (Q2).

## Dictation integration (bridge to P3)

The command input accepts dictation via the existing STT path: focus the input, hold the dictation key, speak
the instruction, it transcribes into the box, Enter runs it. This is the minimal, safe first taste of
"control by voice." **P3** then extends it to fully hands-free (wake-word/mode → speak command → auto-run →
insert) — but P1 deliberately keeps a human pressing Enter (no surprise auto-execution).

## What changes in code (rough, for when we build)

- Backend: a `"custom"` action in `ai_transform` (instruction → system prompt); extend `system_prompt_for`
  for the P2 named actions. No new engine work — it all rides the seam.
- Frontend (`ReviewOverlay.tsx`): add the command input + pins + "more actions" popover; a result view that
  handles both the proofread panel and the before→after AI view (already exists).
- Reuse: Apply/Copy/clipboard-restore, model picker, caching — all already built.

## Open questions (resolve before building)

- **Q1 — routing:** start model-direct (A), or invest in the tiny classifier (B) up front? (Recommend A.)
- **Q2 — Translate UX:** free-form ("translate to X") only, or a small language dropdown for the common ones?
- **Q3 — context suggestions:** worth the heuristics in P1, or defer? (Detecting language / email-shape is
  cheap; detecting intent is not.)
- **Q4 — pins:** fixed defaults for P1, or user-configurable pins from the start?
- **Q5 — result view:** for multi-paragraph outputs (Expand/SOAP), do we need a diff/side-by-side, or is the
  current before→after enough?

## Explicitly NOT in P1

Dictation command *mode* (P3), vision (P4), user-configurable pins (maybe), the tiny intent classifier (B).
Keep P1 to: the command input (typed + dictated-into), model-direct routing, and 2–3 pins.
