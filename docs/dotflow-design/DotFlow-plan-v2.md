# DotFlow — Build Plan v2

## One-line thesis

**DotFlow is dictation + dot phrases + AI cleanup that works in any text field, with swappable vocabulary packs and phone-as-mic. One engine, four audiences.**

Speech-to-text is not the product. Phrase expansion is not the product either — that's solved (Espanso, Beeftext, TextExpander all do it). The product is **dictation that triggers your reusable language**: the workflow around voice-native insertion, reusable phrases, live feel, and good defaults — packaged so people who repeat themselves all day (or talk faster than they type) actually use it every day.

What makes it voice-native and frictionless (vs. typing-first expanders): the user can **speak** the trigger, **type** it, **search** it, later **tap** it from a phone, and **combine dictated text + a template in one flow** — all in whatever field they're already using. Expansion is solved; making it voice-native and frictionless is the wedge.

Positioning sentence:

> **Dictation that triggers your reusable language. Speak, expand, clean up — in any field.**

---

## 1. The core insight (what changed in v2)

DotFlow is **one engine with vocabulary/phrase packs**, not a clinical tool and not a generic tool. The same V1 — push-to-talk + phrases + cleanup + paste-anywhere — serves every audience. The _only_ thing that forks per audience is a data pack (vocabulary bias + starter phrases), never the code.

| Audience                  | Why they want it                                                                                            | Accuracy bar                        | How you reach them                         | Pay?         |
| ------------------------- | ----------------------------------------------------------------------------------------------------------- | ----------------------------------- | ------------------------------------------ | ------------ |
| **Coders / AI-IDE users** | Talk faster than typing; dictate prompts into Cursor / Claude Code / chat boxes; LLM tolerates sloppy input | **Low** (model forgives errors)     | GitHub, Show HN, X — **free distribution** | BYO-key, yes |
| **Attorneys / admin**     | Heavy boilerplate, repetitive correspondence, bad typists                                                   | Medium                              | Hand-sold, referrals                       | High         |
| **Clinicians**            | Medical-term accuracy, A/P templates, everywhere-not-just-the-visit                                         | **High** (drug names must be exact) | Demos, trust, word-of-mouth                | Highest      |
| **General / home**        | Hate typing, bad laptop mic                                                                                 | Medium                              | Paid marketing (later, can't afford now)   | Low          |

**Launch surface = coders.** They're the only segment reachable with zero marketing budget, they tolerate an Electron beta, they're comfortable BYO-key, and their accuracy bar is the easiest to clear. They validate the engine in the wild and generate testimonials. _Then_ layer vocab packs to walk uphill into attorneys and clinicians where the money is.

**Build rule that protects this:** build insertion against the _hardest_ target (web EMR contenteditable fields), launch to the _easiest_ one (code editors). If insertion survives a paste-blocking EMR box, it works everywhere. The reverse is not true.

**Two architecture decisions locked in by the ASR/stack research (see companion doc):** (1) **fork Handy** — MIT-licensed Tauri+Rust app that already wires Parakeet v3 + Silero VAD + enigo injection, ~80% of the V1 engine; (2) **inject text, don't paste** — `enigo.text()` keystroke injection is the primary insertion path (clipboard is fallback), and text is injected **per clause as you speak** (VAD-segmented), which is what makes it _feel like Dragon_ instead of a dead record-then-paste wrapper.

---

## 2. Why this idea is strong

The "tech exists, people hate using it" thesis fits exactly. The pieces all exist — local Whisper/faster-whisper, hotkey voice-to-cursor apps, text expanders, browser expansion extensions, phone-as-mic tools, streaming transcription, BYO-key AI cleanup — but the experience is fragmented. Users today choose between:

- **Dragon** — powerful, accurate, deep macros; expensive, dated, bloated.
- **Built-in dictation** — free; not customizable, no phrases.
- **Voice-to-cursor apps (OpenLess, Whispo, unmute, VoiceFlow)** — good dictation; thin phrase/macro workflow.
- **Text expanders (Espanso, Beeftext, TextFast)** — good expansion; typing-first, config-heavy.
- **Phone-as-mic tools** — audio routing, not a real dictation workflow.
- **Ambient AI scribes (Abridge, DAX, Suki, Freed)** — write the whole note; clinical-only, visit-bound, cloud-only, expensive.

DotFlow's wedge: **voice-first text expansion that works in every field.** Say it, type it, tap it, or search it — and your reusable text appears anywhere. It sits _next to_ the ambient-scribe wave, not against it: scribes own the visit; DotFlow owns the other 50 text boxes a professional types into all day (portal messages, refill notes, prior auths, emails, IDE prompts, legal correspondence).

---

## 3. The moats — in priority order (corrected)

Speech recognition is not the moat. And vocabulary, despite being cheap and clever, is **not the _near-term_ moat** — that was an over-claim. The honest ordering:

**The near-term moat (what actually locks users in early):** the user's **own phrase library + daily habit + insertion reliability + the live feel.** Once someone has built personal shortcuts, wired their hotkey reflexes, and trusts that text lands correctly every time, switching is annoying — and it's annoying from day one, before any vocabulary pack exists. This is what you protect first. Priority stack for the whole product:

> **1. Feel → 2. Reliability → 3. Phrase library → 4. Voice aliases → 5. Cleanup → 6. Packs → 7. Phone.**

**The upsell moat (later, and real):** vocabulary biasing. Generic ASR mangles domain terms — `metoprolol`, `levothyroxine 88 mcg`, `voir dire`, `useEffect`. A vocabulary pack that biases transcription gives accuracy competitors can't cheaply match. But it's the thing people _upgrade into once already hooked_, not the thing that hooks them. Treat it as the paid-tier differentiator, not the launch wedge.

**On the medical vocabulary source — license caution.** UMLS Metathesaurus is free but has redistribution restrictions (requires a UMLS Terminology Services / NLM account; SNOMED/ICD layered terms carry their own terms). You generally **cannot ship the raw Metathesaurus in a binary.** The safe path is a _derived surface-form list_ (terms only, no codes/relationships), usually permissible — verify before baking it in. **RxNorm is more permissively licensed and covers drug names**, ~80% of the medical-dictation accuracy pain for cheap. Start there.

**Critical technical caveat:** vocabulary injection has to work with your ASR engine. Local Parakeet/`faster-whisper` support real custom vocab / `hotwords`; a **post-ASR correction pass** (fuzzy-match against your term list) is provider-agnostic and the realistic production layer. This is wired into the cleanup pipeline, not a separate subsystem.

---

## 4. The product: four core actions

**1. Dictate** — hold hotkey, speak, insert into the active field. Modes: push-to-talk, toggle, live preview, paste-on-release.

**2. Expand** — type or say a shortcut: `.copd`, `.refund`, "insert COPD plan", "insert refund response".

**3. Transform (explicit only)** — "clean this up", "fix punctuation only", "make concise", "format as A/P", "rewrite as professional email", "convert to bullets". **Raw dictation by default; AI rewriting only when requested** (the unmute principle).

**4. Remote** — phone as mic + macro pad. Hold phone button → dictate to desktop. Tap a favorite → insert on desktop.

---

## 5. Build order (revised)

The big change from v1: **insertion reliability and vocabulary biasing are validated in a week-one spike _before_ any product UI**, and the **browser extension is pulled forward** because for the highest-value users the browser field _is_ the field.

### Phase 0 — Week-one validation spike (throwaway code, no UI)

Prove the two things that can kill the product before building anything pretty.

- **Insertion reliability — injection-first.** Test `enigo.text()` keystroke injection (PRIMARY) into: a code editor (easy), Gmail/Slack web (medium), and a web contenteditable EMR-style field or paste-restricted box (hard). Measure the "Dragon feel" — does text pop in place, instantly, in the field's own formatting, clipboard untouched? Clipboard-paste is tested only as the **fallback** for fields where injection fails and for very long blocks. This reverses the original clipboard-first ordering — injection is what makes it feel like Dragon. If injection survives the hard target, the core thesis is proven.
- **Vocabulary accuracy.** Dictate 10 real medication names + an A/P paragraph through (a) raw Groq/OpenAI and (b) RxNorm-biased or post-ASR-corrected pipeline. If biasing fixes the drug names and raw doesn't, the Moat-B thesis is proven in an afternoon. If it doesn't move the needle, you know the "cheaper Dragon" bet rests on workflow alone — important to learn _now_.

**Gate:** do not proceed to Phase 1 until keystroke injection feels Dragon-like in the hard target and vocab biasing measurably helps.

### Phase 1 — Narrowest useful desktop app (fork Handy, local-first)

Hotkey → speak → text appears live in the active field, like Dragon.

- Fork **Handy** (MIT) as the base: Tauri+Rust, **local Parakeet v3 int8** + Silero VAD + enigo injection already wired. No API, no key, no cloud — kills API cost from day one.
- **Injection-first insertion** (`enigo.text()` primary, clipboard fallback for huge blocks). Ship a **"reliable paste" fallback mode** for browser/Electron fields where injection is flaky — FluidVoice (mature Mac app) needed exactly this despite using accessibility-API injection, so treat it as required, not optional.
- **Clause-level continuous injection** — VAD segments at micro-pauses, warm Parakeet transcribes each, injects immediately. Text accumulates _as you speak_ = the live feel.
- **Floating live overlay** — waveform + in-flight partial so there's never an "is it listening?" gap.
- **Punctuation toggle: auto | spoken | raw** (see below) — rides the deterministic cleanup pipeline.
- Global push-to-talk + toggle hotkeys; warm-loaded model (no cold lag); settings.
- **Success:** usable all day, _feels like Dragon_, in Gmail, Word, ChatGPT/Claude, Cursor, Slack, and a web EMR-style field — fully offline, $0 API.

### Phase 1.5 — Punctuation modes (small, high-value for Dragon-trained users)

- **Auto** (default): model's native punctuation + capitalization (free from Parakeet/Canary).
- **Spoken** (Dragon-style): user says "period / comma / new line / new paragraph"; engine strips model punctuation and maps spoken tokens → marks via a <1ms command table. Optional rule-based correction if the user mixes modes (no LLM, off by default for verbatim purists).
- **Raw**: no punctuation processing — for coders dictating into an IDE who don't want `camelCase` mangled.

### Phase 2 — Phrase engine (the wedge — before over-polishing dictation)

- Phrase library UI (add/edit/delete), phrase search palette, insert phrase.
- Dot-trigger expansion (`.copd`), voice-alias matching after dictation, import/export JSON, local SQLite.
- **Success:** "insert COPD plan" (spoken) or `.copd` (typed) inserts the same saved phrase in any field. First real "aha."

### Phase 3 — AI cleanup / command mode (explicit)

- Cleanup selected text / last dictation; punctuation-only, professional, concise, A/P, "do not change meaning / add no facts".
- Triggered by hotkey on selection, command palette, or spoken instruction.
- **Success:** rough dictation → usable text with one hotkey, without leaving the field.

### Phase 4 — Browser extension (pulled forward — load-bearing, not optional)

For coders (web IDEs, AI chat) and clinicians (web EMRs), the browser field _is_ the field; even keystroke injection can be weak in some web contenteditable. The extension makes insertion native and reliable there.

- Detect textareas / contenteditable; dot-phrase candidate menu; selected-text cleanup; insert into web fields directly; connect to desktop over local WebSocket with standalone fallback.
- **Success:** smoother in Gmail, ChatGPT/Claude, web EMRs/portals, and browser IDEs than a desktop-only tool.

### Phase 5 — Phone web remote (great demo, not the daily driver)

Web remote first — **no native app.** QR pair → phone push-to-talk → audio to desktop/local → insert into active field; phone shows favorite-phrase buttons + search.

- **Reality check:** this is the 20-second demo that sells DotFlow, but phone-as-mic adds latency and pairing friction; most desk users will just use the hotkey. Keep it lean; don't let it eat engineering oxygen.

### Phase 6 — Optional cloud BYO-key + true streaming + LLM transforms

Local-first is already the default (Phase 1). This phase adds the _opt-in_ extras:

- **Cloud BYO-key** transcription/cleanup for users who want frontier-quality rewrites and don't care about cost/privacy (engine-agnostic router makes this config, not a rewrite).
- **True-streaming "live mode" — DEFERRED, conditional.** The bet is that **Parakeet TDT clause-level injection (Phase 1) feels live enough.** If, after using your own build, the cadence still feels chunky on long-form, add a streaming model — and try **Parakeet Flash first** (English-only, "feels INSTANT," proven in FluidVoice at <100ms on Apple Silicon), with **Nemotron** as the multilingual/CPU-benchmark alternative. Verify each model's license before shipping. Don't build the streaming pipeline prematurely — you'll know within a week of real use.
- **Local LLM transforms** (Ollama/LM Studio, Qwen 2.5 1.5B) for explicit "rewrite / make professional / A/P format"; cloud as optional quality upgrade.
- Provider recipes (fast cloud / private local / medical vocab / low-cost).
- **Marketing line:** _Your voice. Your phrases. Your models. Your computer._

### Later

Mac app → native phone apps → team/shared phrase packs → pack marketplace.

---

## 5b. Parking lot — Voice-Addressable Targets (future, not V1)

A genuinely differentiating future feature. **Not** a V1 commitment — captured here so it shapes architecture decisions early without bloating scope.

**What it is:** the user tags a field / pane / window / region once; the app remembers how to find it again; then voice commands like "go to terminal two," "jump to assessment," "the chat box" _move focus_ to that target. This is the missing half of voice control — today DotFlow puts text where the cursor is; this lets the voice move the cursor to a named place. It turns dictation into actual hands-free operation, which no Whisper wrapper does.

**Why it fits DotFlow specifically:**

- **Vibe-coding:** tag "left terminal" / "right terminal" / "editor" / "chat" and bounce between them by voice. Nobody sells this.
- **EMR:** jump between named regions of a note ("assessment," "plan," "meds") without the mouse.
- It deepens the moat: now the user accumulates _targets_ + _phrases_ + _workspaces_ — switching cost climbs further.

**Staging (re-identification difficulty drives the order):**

1. **Windows / panes first (achievable).** OS accessibility APIs (Win UI Automation, mac AXUIElement) expose window handles/titles; focus by handle. "Terminal two" → stored window ref → activate. This is the _same accessibility layer_ as the premium injection path (research §11a method 2), so it's architecturally adjacent, not a new universe. Nails the vibe-coding terminal-switching demo.
2. **Fields-within-app second (best-effort).** "The assessment field" has no stable address — web DOM regenerates, EMRs nest iframes with dynamic IDs, a tag saved today may not resolve tomorrow. This lives in the **browser extension** (anchor to DOM with fallback heuristics: nearby label text, role, position, surrounding text) and is accepted as imperfect. Don't promise field-level until you've felt how flaky re-identification is in real EMRs.

**Saved workspaces (the commercial framing):** a "workspace" = a named set of tagged targets + its phrase pack + punctuation mode. "Open charting workspace" loads EMR targets + clinical pack; "open coding workspace" loads terminal/editor tags + code pack. Ties together targets + packs + profiles into one switchable context. Also gives the **phone remote a real job** — phone becomes a labeled control surface, tagged targets as buttons, tap "left terminal" → focuses it on the desktop. A stronger phone rationale than "phone as mic."

**Safety caveat to design for now:** voice-driven _focus switching_ raises the stakes on misrecognition. A mis-dictated word is a visible typo; "go to terminal two" focusing the wrong window — then dictating a destructive command into it — is worse. Targets that _execute_ (terminals, anything that runs on Enter) need a confirmation or a clear focus indicator before injection, unlike plain dictation fields.

**Architectural implication for V1:** none required, but the **accessibility-API insertion path** (research §11a method 2) is the shared foundation for both premium injection _and_ window-level targeting — so when/if you build that path, you get the groundwork for this nearly free. Keep target/workspace as a clean data model in mind; don't build it yet.

---

## 5c. Banked offshoot — Voice front-end for an agent (OpenClaw etc.) — V2+, not committed

_Filed only to remember the core was designed to allow it. Not a roadmap item._ **Note: FluidVoice already ships a "Command Mode" (voice → launch apps / run shortcuts / system actions) on Mac — so voice-driven computer control is validated as desirable but is no longer novel. DotFlow's version would differentiate on cross-platform + the accurate vocab layer + agent integration (OpenClaw), not on being first.**

Same core engine (accurate local voice → parse → route), pointed at a **different output sink**: instead of injecting text into a field, route the parsed command to an autonomous agent. **OpenClaw** is the obvious target — MIT-licensed, local-first, runs as a Gateway on `127.0.0.1:18789`, model-agnostic, skill/Markdown-based (compatible with how you already work), and exploding in adoption (OpenAI-backed foundation as of early 2026). DotFlow becomes the _accurate voice layer_; OpenClaw becomes the hands. "Control the computer completely hands-free by voice."

**Why it's just banked, not planned:**

- **Different risk profile entirely.** DotFlow's whole thesis is a deterministic, trustworthy typing tool — exact words, visible, undoable. Piping voice into an agent that runs shell commands inverts that: a misrecognized _command_ with system access is far worse than a visible typo. This is the §5b executing-target safety problem at its most dangerous.
- **If ever built, it's opt-in and gated:** a separate "agent mode" surface (distinct hotkey), a hard confirm-before-execute gate, a visible "this will run an action" indicator. DotFlow stays a deterministic typing tool by default.
- **The positioning that keeps it tethered to the moat:** not "DotFlow controls your computer" (everyone will bolt generic Whisper onto OpenClaw) but **"DotFlow is the accurate, domain-vocab voice layer that makes agent control trustworthy."** The RxNorm/term-correction + clause accuracy is what makes voice-driven agent commands not terrifying — that's the edge.
- **Moving target:** OpenClaw is months old, renamed twice (Clawdbot → Moltbot → OpenClaw), just changed governance. Don't make it load-bearing until both it and DotFlow have settled. V2+, post-monetization, revisit-only.

**Architectural implication for V1:** none. The same parse-and-route layer that picks "inject here" vs "expand phrase" is where an "→ send to agent" branch would later attach. Keep the command-routing layer clean and sink-agnostic and this stays cheap to add later.

---

## 6. Stack (revised by the ASR/stack research — see companion doc)

**This supersedes the earlier "Electron + cloud-Whisper-first" plan.** The model/stack research changed the recommendation: there's a proven, MIT-licensed, local-first base to fork, so building Electron-from-scratch-on-cloud-Whisper would be slower _and_ feel worse.

**Recommended base: fork Handy (MIT).** Tauri 2 (Rust + React/TS), cross-platform Win/mac/Linux, with **local Parakeet v3 int8 + Silero VAD + enigo injection already wired.** That's ~80% of the V1 engine, commercially licensed, and it's local-first so there's no API cost. Layer DotFlow's own code (phrase engine, RxNorm correction, punctuation toggle, clause-injection cadence, live overlay) on top.

**Engine internals:** `sherpa-onnx` (Apache-2.0) runs Parakeet/Canary/Nemotron/Whisper across all platforms · Parakeet TDT v3 int8 (CC-BY-4.0) default model · Silero VAD (MIT) for clause segmentation · `enigo` (MIT) for text injection · SQLite for phrases · local WebSocket for extension + phone.

**Why not Electron-from-scratch:** the original plan's Electron+Python-sidecar route is more work, heavier install, and you'd be reimplementing exactly what Handy already gives you (warm model loading, VAD, injection, cross-platform hotkeys). Fork beats build here.

**Watch-out:** global hotkeys + cross-app injection is still where platform pain lives, but Handy has already solved most of it on Win/mac/Linux — that's the point of forking it. Validate injection feel in the Phase-0 spike regardless.

---

## 7. Repo landscape — verified licenses (see companion research doc for full table)

Build a clean commercial core. The good news from the license audit: **the entire core stack is MIT/Apache/CC-BY — zero copyleft.** Avoid copying code only from the GPL/AGPL dictation apps.

| Repo                                     | Borrow                                                                       | License (verified)                        |
| ---------------------------------------- | ---------------------------------------------------------------------------- | ----------------------------------------- |
| **Handy** (`cjpais/Handy`)               | **the base — fork it.** Tauri+Rust, local Parakeet+VAD+enigo, cross-platform | **MIT** ✅ take freely                    |
| **OpenWhispr** (`OpenWhispr/openwhispr`) | provider router, BYO-key, auto-learn dictionary                              | **MIT** ✅ take freely                    |
| **enigo**                                | text/keystroke injection (primary insertion)                                 | **MIT** ✅ take freely                    |
| **sherpa-onnx**                          | the ASR engine                                                               | **Apache-2.0** ✅ take freely             |
| **Silero VAD**                           | clause segmentation                                                          | **MIT** ✅ take freely                    |
| **Parakeet v3 / Canary 180m**            | the models                                                                   | **CC-BY-4.0** ✅ attribute                |
| **macparakeet / speak2**                 | deterministic <1ms cleanup, phonetic dictionary, live overlay                | verify each LICENSE; reimplement patterns |
| **TextFast / ChromeAutoTextExpander**    | browser-field expansion                                                      | MPL-2.0 / verify                          |
| **Espanso / hallelujahIM**               | expander concepts                                                            | GPL-3 — **concepts only** ⚠️              |
| **VoiceTypr / Whispo**                   | (reference)                                                                  | AGPL-3 — **concepts only** 🚫             |

**Bottom line:** fork Handy + lift OpenWhispr/enigo patterns + sherpa-onnx + Silero + Parakeet = a 100% commercially-shippable, zero-copyleft foundation. Your phrase engine, RxNorm layer, punctuation toggle, and live-injection cadence are original code on top.

---

## 8. Starter packs (ship day one — packs make it feel alive)

Packs are **editable templates, not advice.**

- **Coder (launch pack):** `.fix` "fix the bug where", `.refactor`, `.test` "write a test for", `.explain`, `.commit`, `.pr`, `.todo` — plus a **code-term vocab bias** (common API/library names) so prompt dictation reads cleanly.
- **General:** `.ty` `.fu` `.apology` `.summary` `.todo` `.meeting` `.emailwarm` `.emailconcise` `.sig`
- **Clinician (the accuracy-moat pack):** `.normalpe` `.rosneg` `.copd` `.chf` `.dm2` `.htn` `.ckd` `.fall` `.discharge` `.followup` `.medrec` `.labsreviewed` + **RxNorm-biased vocabulary**.
- **Attorney/admin:** `.review` `.attached` `.perourcall` `.confirm` `.requestdocs`
- **Support/sales:** `.refund` `.delay` `.followup` `.intro` `.pricing` `.schedule` `.escalate`

---

## 9. Pricing (simplified three-tier — floor held at $10)

Cleaner and easier to understand than segmenting into "Clinical/Legal tier" on day one. Three named tiers + a free funnel + a launch lifetime deal. **Floor is $10, not $5** — $5 reopens the race-to-free problem (too cheap to fund reaching paying audiences, barely above free built-in dictation). Don't hard-segment by profession yet; ship the structure, watch which packs people actually want.

- **Free** — push-to-talk dictation, local/BYO transcription, small phrase cap (~10), insert into any field, basic punctuation modes. (Funnel + coder goodwill.)
- **Basic — $10/mo** — for regular users: everything in Free + a reasonable phrase cap, basic phrase expansion, all punctuation modes.
- **Pro — $15/mo** — for daily users: unlimited phrases, voice aliases, phrase search palette, AI cleanup/transforms, browser extension, custom/BYO providers, import/export, phone web remote when ready.
- **Premium Packs — $20+/mo** — Pro + domain accuracy: medical pack (RxNorm-biased), legal/admin pack, coder pack (if premium-worthy), specialty vocab bias/correction, starter phrases, profile-specific cleanup styles, local-private setup docs. Anchor against Dragon Medical (~$500+) and ambient scribes — a fraction of the price _and_ works everywhere.
- **Launch Lifetime deal — ~$150 one-time** — for early adopters at launch (coders love this; converts the free-distribution crowd into committed users and pulls cash + validation forward). Cap it (first N buyers) and **scope it to the local/on-device product** — don't promise lifetime _cloud_ services you'd have to fund forever.
- **Team (later)** — ~$12–15/user/mo: shared libraries, admin packs, sync.

**Why this works:** Free + Basic hook the broad/coder audience who find you for free; Pro captures daily users; Premium Packs capture the domain-accuracy willingness-to-pay where Dragon/scribes set the anchor. **Validate which packs people want before building them all** — ship one test pack, watch demand, then build the legal/specialty packs people actually ask for.

---

## 10. Honest risks (name them so they don't bite)

- **There's a strong, fast-moving competitor to watch: FluidVoice** (`altic-dev`, Mac-only, GPLv3, **5.8k★, 35 releases, v1.6.1 June 2026** — roughly doubled since first noted). It's the further-along version of this and it **already ships three things that were on your "novel idea" list:** live-preview overlay (near-zero-delay word-by-word), **Command Mode** (voice controls the Mac — the shipping version of your Voice-Addressable-Targets/OpenClaw offshoot), and **Write/Rewrite mode** (select-and-transform by voice). So those ideas are _validated as desirable but no longer novel_ — on Mac. Three honest takeaways: (1) it _validates_ the architecture (live feel achievable, model strategy right, injection-needs-paste-fallback real, live-preview-in-overlay is the pattern); (2) DotFlow's defensible space is **cross-platform (it's Mac-only, Windows only on a waitlist — no Swift head start there), the phrase/dot-phrase wedge (it has none), and domain vocab packs (it has none)**; (3) its **open-core model is a template** — free open-source app, _separately-maintained closed-source_ enhancement model (Fluid Intelligence). Mirror it: free/open core, paid closed vocab packs. **Don't try to out-Mac them; win on Windows + phrases + packs, and frame DotFlow as the phrase/vocab tool that dictates, not a dictation tool.**
- **"Cheaper Dragon" ≠ home run by itself.** Dragon's stickiness is accuracy + custom vocab + deep macros, not just features. If you're cheaper but 90% as accurate, the 10% sends clinicians back. The home run is _cheaper Dragon that's actually accurate on domain terms_ — which is exactly what the vocab work buys. Price is the hook; vocabulary is why they stay.
- **Insertion is the real engineering — and it's injection, not paste.** Keystroke/text injection (`enigo`) into secure & contenteditable fields is where the Dragon feel is won or lost. Forking Handy gives you a head start (it already does this on 3 platforms), but injection feel in web/Electron fields is still the Phase-0 thing to prove.
- **"Feels live" is non-negotiable and is a cadence problem, not a model problem.** Handy feels dead because it injects once at the end. DotFlow injects per clause as you speak. If you skip continuous injection, it'll feel like every other Whisper wrapper no matter how good the model is. This is V1 core.
- **The moat is the habit, not the vocabulary (corrected).** The near-term lock-in is the user's own phrase library + daily reflexes + insertion reliability + live feel — present from day one. Vocabulary packs are the _upsell_ people pay to grow into once hooked, not the launch wedge. Protect feel and habit first; sell accuracy second. Priority: feel → reliability → library → aliases → cleanup → packs → phone.
- **Phrase expansion is solved — don't claim it as the innovation.** Espanso/Beeftext/TextExpander already do triggers. The wedge is making it _voice-native and frictionless_ (speak/type/search/tap the trigger, combine dictation + template in one flow, in any field). Positioning that claims to invent expansion invites "just use the free one."
- **Multi-audience → build-for-all, ship-for-none.** The same V1 serves everyone; resist audience-specific UI until one segment pulls. Vocabulary is a _data pack_, not a code fork — keep it that way.
- **Ambient scribes are the marketed competitor for clinicians.** Don't fight them; own the everywhere-fields lane they don't touch.
- **"A bit of marketing is enough" is true only for coders.** Every other segment needs trust, demos, or budget. That's _why_ coders launch first.

---

## 11. Build path to completion (main features → shippable product)

A sequenced path from zero to a paid-ready product. Phases gate on _feel_, not calendar. Rough effort assumes solo, evenings/weekends, forking Handy.

**Milestone 0 — Validation spike** _(a few days, throwaway)_

- enigo injection feel across Cursor / Gmail / web-EMR / Word.
- Parakeet v3 int8 latency on your real hardware (target: text starts appearing < ~1s after you speak).
- RxNorm vocab correction: biased vs raw on 10 med names.
- **Gate:** injection feels Dragon-like in the hard target + vocab biasing measurably helps. → proceed.

**Milestone 1 — The core engine that feels like Dragon** _(~2–3 weeks)_ — **this is the make-or-break**

- Fork Handy; confirm local Parakeet + VAD + enigo runs on your machines.
- Flip to **injection-first** insertion (enigo primary, clipboard fallback for huge blocks).
- **Clause-level continuous injection** + warm model + **live overlay** (waveform + in-flight partial).
- **Punctuation toggle** (auto / spoken / raw).
- **Basic preview + fast undo** — voice insertion that drops the wrong text with no quick undo feels dangerous; one-keystroke undo of the last insertion is a V1 safety requirement, not a polish item.
- **Deliverable:** dictate into any field, text appears live as you speak, feels like Dragon, fully offline, $0 API. _If this milestone doesn't feel right, nothing else matters — stop and fix it (or test Nemotron) before building further._

**Milestone 2 — The wedge: phrases** _(~2 weeks)_

- Phrase library (SQLite) + search palette + dot-trigger expansion + voice aliases.
- **Parse-and-expand-before-inject** so phrases land as one clean block; command-buffer for triggers spanning a pause.
- Custom dictionary / RxNorm correction wired into the cleanup pipeline.
- **Deliverable:** "insert COPD plan" (spoken) or `.copd` (typed) drops the template in place, live. First real "aha."

**Milestone 3 — Coverage where the buyers live: browser extension** _(~1–2 weeks)_

- Chrome extension: contenteditable detection, dot-phrase menu, native web-field insertion, local-WebSocket link to desktop + standalone fallback.
- **Deliverable:** reliable in Gmail, ChatGPT/Claude, web EMRs/portals, browser IDEs.

**Milestone 4 — Cleanup & transforms** _(~1 week)_

- Deterministic cleanup covers most cases (punctuation, caps, dictionary). Explicit transforms ("clean up / concise / A/P / professional") via local LLM (Ollama) with cloud BYO-key as optional upgrade.
- **Deliverable:** rough dictation → usable text with one hotkey, no field-leaving.

**Milestone 5 — Ship to coders (first real launch)** _(~1 week polish + launch)_

- Coder phrase pack + code-term vocab. Installer that "just works" (the local-model packaging tax — budget for it). Onboarding, settings, import/export.
- **Deliverable:** Show HN / GitHub / X — "hands-free vibe coding." Free tier live. Collect testimonials.

**Milestone 6 — Monetize: clinical/legal vocab tiers** _(~2 weeks)_

- RxNorm-biased medical pack + legal pack as the paid Clinical/Legal tier. Free/Pro/Clinical pricing live. Local/private-mode emphasis for trust.
- **Deliverable:** paying users where willingness-to-pay is highest.

**Milestone 7 — Phone web remote** _(~1 week)_

- QR pair → phone push-to-talk → insert into desktop active field; favorite-phrase buttons. The demo-seller; keep lean.

**Then — optional / conditional:**

- Nemotron true-streaming _only if_ Milestone-1 cadence felt chunky.
- Mac fast path (MLX/FluidAudio). Native phone apps. Team/shared packs + marketplace.
- **Voice-Addressable Targets** (§5b): windows/panes first, then fields, then saved workspaces.

**Critical-path summary:** Milestones 0→1→2 are the product. Everything after is coverage, monetization, and reach. If you only ever shipped 0–2, you'd have a local, Dragon-feeling, phrase-expanding dictation tool that already beats every Whisper wrapper. Protect Milestone 1's _feel_ above all — it's the whole differentiator.

---

## 11c — The human sequence (how to actually run it)

The build path above is _what_ to build; this is _how_ to not fool yourself while doing it:

1. **Prove the feel before the UI** — the Phase-0 spike exists so you don't build a month of product on top of insertion that feels wrong.
2. **Be your own first user** — dictate into your real IDE/EMR work all day. If it doesn't feel like Dragon _to you_, it won't to anyone. This is the only test that matters for Milestone 1.
3. **The 20-second demo is the product** — click any field → hold hotkey → "Patient improving. Insert COPD plan. Follow up pulmonary." → text appears _live as you speak_, template pops in place. If you can't demo that in 20 seconds, the feel isn't there yet.
4. **Launch where it's free to launch** — coders, via Show HN / GitHub / X. Don't spend money you don't have marketing to audiences that need trust.
5. **Let packs be the moat** — the library + vocabulary + (eventually) workspaces are what make leaving annoying.
6. **Walk uphill to the money** — coders validate, clinicians/attorneys pay. The vocab pack is the bridge.

---

## 12. Final product statement

**DotFlow — Dictation that triggers your reusable language.**

> Hold a key, talk, and text appears live in any field. Say "insert follow-up" or type `.fu` and your saved phrase drops in. Local-first and private, BYO key or your own models, Dragon-style productivity without Dragon pricing — and it works everywhere, from your IDE to your inbox to your EMR.

Own one sentence:

> **The voice-native way to trigger your reusable language — for people who repeat themselves, or talk faster than they type.**

Launch it on the people who'll find it for free (coders), monetize the people who'll pay the most (clinicians, attorneys), and let the vocabulary packs be the moat nobody can cheaply copy.
