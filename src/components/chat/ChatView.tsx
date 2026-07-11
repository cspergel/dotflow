import {
  useState,
  useRef,
  useEffect,
  useCallback,
  useMemo,
  type MouseEvent,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import {
  Send,
  Square,
  Plus,
  MessageSquare,
  Trash2,
  PanelLeftClose,
  PanelLeftOpen,
  Copy,
  Check,
  Upload,
  Mic,
  Brain,
  Paperclip,
  FileText,
  X,
  Loader2,
} from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { commands, type LocalModelInfo } from "../../bindings";
import { sanitize, parseThinking } from "./chatText";
import { ChatMarkdown } from "./ChatMarkdown";
import { useChatDictation } from "./useChatDictation";
import {
  loadConversations,
  saveConversations,
  titleFrom,
  newId,
  estimateTokens,
  loadReason,
  saveReason,
  reasonSuffix,
  OPEN_KEY,
  type ChatMsg as Msg,
  type Conversation,
} from "./chatStore";

// A light default persona so the local model is a genuinely helpful assistant rather than over-refusing.
// It is prepended to the request only — never shown in the transcript or stored in history.
const SYSTEM_PROMPT =
  "You are DotFlow, a helpful, knowledgeable assistant running locally and privately on the user's own " +
  "computer. Give clear, direct, useful help. Provide general educational information on any topic — " +
  "including health, medical, legal, and financial subjects — to help the user understand, plan, and draft. " +
  "You are not a substitute for a licensed professional; briefly suggest consulting one for personal " +
  "decisions, but do NOT refuse to give general information or assistance.";

// One-tap clinical presets for an attached document. Each is a full instruction sent to the summarizer.
const HPI_PROMPT =
  "Using only the attached chart, write a comprehensive, chronological History of Present Illness for a " +
  "skilled nursing facility admission: the reason for admission, the hospital and ICU course, procedures, " +
  "complications, key treatments and antibiotics, and the current functional, cognitive, and swallowing/diet " +
  "status at transfer. Complete sentences and paragraphs, covering the whole stay from admission to discharge.";
const PROBLEM_LIST_PROMPT =
  "From the attached chart, produce a COMPLETE assessment/problem list for a skilled nursing facility " +
  "admission. Include EVERY problem: the acute medical diagnoses and complications from the hospital course, " +
  "the chronic conditions being managed, AND the functional/therapy problems. Do not stop until every " +
  "documented or clearly-implied problem is listed. For each: the problem name tagged [Documented] or " +
  "[Suspected]; Evidence (the findings that support it); Plan; and ICD-10 (suggested, verify).";

// User-defined preset buttons (personal, stored locally). Each is a named one-tap prompt run against the
// attached document(s) — so a clinician can build their own library (SBAR, med rec, discharge summary, …).
const PRESETS_KEY = "dotflow.chat.customPresets";
type CustomPreset = { id: string; label: string; prompt: string };
function loadCustomPresets(): CustomPreset[] {
  try {
    const raw = localStorage.getItem(PRESETS_KEY);
    if (raw) {
      const arr = JSON.parse(raw);
      if (Array.isArray(arr))
        return arr.filter((p) => p && p.id && p.label && p.prompt);
    }
  } catch {
    /* ignore malformed storage */
  }
  return [];
}
function saveCustomPresets(presets: CustomPreset[]) {
  try {
    localStorage.setItem(PRESETS_KEY, JSON.stringify(presets));
  } catch {
    /* best-effort */
  }
}

// Merge several read/OCR'd PDFs into ONE document with per-file headers, so the model treats 15 files as a
// single record (the "combine into one record" mode). Order preserved; the name is the file count when >1.
function buildCombinedDoc(parts: { name: string; text: string }[]): {
  name: string;
  text: string;
  files: string[];
} {
  const files = parts.map((p) => p.name);
  const text = parts
    .map((p) => `===== ${p.name} =====\n${p.text}`)
    .join("\n\n\n");
  return {
    name: files.length === 1 ? files[0] : `${files.length} files`,
    text,
    files,
  };
}

function appendToLastAssistant(msgs: Msg[], text: string): Msg[] {
  const last = msgs[msgs.length - 1];
  if (!last || last.role !== "assistant") return msgs;
  return [...msgs.slice(0, -1), { ...last, content: last.content + text }];
}
function replaceLastAssistant(msgs: Msg[], text: string, error = false): Msg[] {
  const last = msgs[msgs.length - 1];
  if (!last || last.role !== "assistant") return msgs;
  return [...msgs.slice(0, -1), { role: "assistant", content: text, error }];
}

/**
 * Offline AI chat, Claude/Codex-style: a conversations rail (persisted to localStorage = cross-session
 * memory), a full-height message area, and a bottom composer. Streams tokens live via the
 * `chat-token`/`chat-done`/`chat-error` events. Backed by the local GGUF model.
 */
export default function ChatView({ active = true }: { active?: boolean }) {
  const { t } = useTranslation();
  const [conversations, setConversations] = useState<Conversation[]>(() =>
    loadConversations(),
  );
  const [activeId, setActiveId] = useState<string | null>(null);
  const [railOpen, setRailOpen] = useState<boolean>(
    () => localStorage.getItem("dotflow.chat.railOpen") !== "false",
  );
  const [messages, setMessages] = useState<Msg[]>([]);
  const [input, setInput] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [models, setModels] = useState<LocalModelInfo[]>([]);
  const [copiedIdx, setCopiedIdx] = useState<number | null>(null);
  const { recording, toggleMic } = useChatDictation(setInput);
  const [ctxTokens, setCtxTokens] = useState<number>(() => {
    const v = parseInt(localStorage.getItem("dotflow.chat.ctx") ?? "8192", 10);
    // Cap at 16k (stable fp16 KV). Bigger fp16 KV for a 9B model risks a CUDA-OOM hard crash on 16 GB; the
    // path to larger contexts is the llama-server sidecar (crash-isolated), not in-process.
    return Number.isFinite(v) && v >= 512 ? Math.min(v, 16384) : 8192;
  });
  const [reason, setReason] = useState<boolean>(loadReason);
  const toggleReason = useCallback(() => {
    setReason((r) => {
      const next = !r;
      saveReason(next);
      return next;
    });
  }, []);
  // Attached PDFs, each already read/OCR'd to text — the source of truth. `attachedDoc` below derives one
  // combined document (with per-file headers) so 15 files become a single record for the model.
  const [docParts, setDocParts] = useState<{ name: string; text: string }[]>(
    [],
  );
  const [attachError, setAttachError] = useState<string | null>(null);
  // Scanned PDFs (no text layer) awaiting OCR: {path, name}. Populated when read_pdf_text reports "scanned".
  const [scannedPdfs, setScannedPdfs] = useState<
    { path: string; name: string }[]
  >([]);
  const [ocrBusy, setOcrBusy] = useState(false);
  // Live OCR progress ("Reading page 12 of 105") for a long scanned chart; null when not OCR-ing.
  const [ocrProgress, setOcrProgress] = useState<{
    done: number;
    total: number;
  } | null>(null);
  const turnIdRef = useRef(0);
  // True while a big-document map/reduce summary is running, so the `doc-summarize-progress` listener knows to
  // update the progress state (and normal chat streams don't).
  const summarizeActiveRef = useRef(false);
  // Live map/reduce progress for the in-flight summary (null when not summarizing) → drives the spinner + label.
  const [summarizeProgress, setSummarizeProgress] = useState<{
    done: number;
    total: number;
    stage: string;
  } | null>(null);
  // Personal, editable preset buttons (persisted to localStorage).
  const [customPresets, setCustomPresets] =
    useState<CustomPreset[]>(loadCustomPresets);
  const [presetEditorOpen, setPresetEditorOpen] = useState(false);
  const [newPresetLabel, setNewPresetLabel] = useState("");
  const [newPresetPrompt, setNewPresetPrompt] = useState("");
  const savePreset = useCallback(() => {
    const label = newPresetLabel.trim();
    const prompt = newPresetPrompt.trim();
    if (!label || !prompt) return;
    setCustomPresets((prev) => {
      const next = [...prev, { id: newId(), label, prompt }];
      saveCustomPresets(next);
      return next;
    });
    setNewPresetLabel("");
    setNewPresetPrompt("");
    setPresetEditorOpen(false);
  }, [newPresetLabel, newPresetPrompt]);
  const deletePreset = useCallback((id: string) => {
    setCustomPresets((prev) => {
      const next = prev.filter((p) => p.id !== id);
      saveCustomPresets(next);
      return next;
    });
  }, []);
  const scrollRef = useRef<HTMLDivElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);

  // Persist the conversation list whenever it changes.
  useEffect(() => {
    saveConversations(conversations);
  }, [conversations]);

  // Slide-out → expand handoff: if the compact quick chat asked to continue here, open that conversation.
  // Runs when the chat becomes active (not just on mount) — ChatView now stays mounted across navigation,
  // so the handoff can't rely on a fresh mount; it fires each time the user switches into the chat.
  useEffect(() => {
    if (!active) return;
    try {
      const openId = localStorage.getItem(OPEN_KEY);
      if (!openId) return;
      localStorage.removeItem(OPEN_KEY);
      const conv = loadConversations().find((c) => c.id === openId);
      if (conv) {
        setActiveId(conv.id);
        setMessages(conv.messages);
      }
    } catch {
      /* best-effort handoff */
    }
  }, [active]);

  // One combined document derived from all attached PDFs (null when none). Everything downstream (summarize
  // routing, presets, context gauge, doc-context injection) consumes this single derived value unchanged.
  const attachedDoc = useMemo(
    () => (docParts.length > 0 ? buildCombinedDoc(docParts) : null),
    [docParts],
  );

  // Live, approximate context-window usage (system prompt + transcript + attached doc + what's being typed).
  const usedTokens = useMemo(() => {
    let sum = estimateTokens(SYSTEM_PROMPT) + estimateTokens(input);
    for (const m of messages) sum += estimateTokens(m.content);
    if (attachedDoc) sum += estimateTokens(attachedDoc.text);
    return sum;
  }, [messages, input, attachedDoc]);
  const ctxPct = Math.min(100, Math.round((usedTokens / ctxTokens) * 100));

  // Attach one or more PDFs: extract each locally and add its text as a part (combined into one record).
  // Text-layer PDFs are read immediately; scanned/image PDFs are queued for OCR.
  const attachPdf = useCallback(async () => {
    try {
      const picked = await open({
        multiple: true,
        filters: [
          { name: "PDF", extensions: ["pdf", "PDF"] },
          { name: "All files", extensions: ["*"] },
        ],
      });
      const paths = Array.isArray(picked)
        ? picked
        : typeof picked === "string"
          ? [picked]
          : [];
      if (paths.length === 0) return;
      setAttachError(null);

      const readParts: { name: string; text: string }[] = [];
      const scanned: { path: string; name: string }[] = [];
      let lastErr: string | null = null;
      for (const path of paths) {
        const name = path.split(/[\\/]/).pop() || "document.pdf";
        const res = await commands.readPdfText(path);
        if (res.status === "ok") {
          readParts.push({ name, text: res.data });
        } else if (/scanned/i.test(res.error)) {
          scanned.push({ path, name });
        } else {
          lastErr = res.error;
        }
      }
      if (readParts.length > 0) setDocParts((prev) => [...prev, ...readParts]);
      if (scanned.length > 0) setScannedPdfs((prev) => [...prev, ...scanned]);
      if (lastErr) setAttachError(lastErr);
    } catch {
      /* dialog cancelled */
    }
  }, []);

  // OCR all queued scanned PDFs (one at a time, with progress), adding each recognized text as a doc part.
  const runOcr = useCallback(async () => {
    if (scannedPdfs.length === 0 || ocrBusy) return;
    setOcrBusy(true);
    setOcrProgress(null);
    try {
      for (const pdf of scannedPdfs) {
        const res = await commands.ocrPdf(pdf.path);
        if (res.status === "ok") {
          setDocParts((prev) => [...prev, { name: pdf.name, text: res.data }]);
          setScannedPdfs((prev) => prev.filter((s) => s.path !== pdf.path));
          setAttachError(null);
        } else {
          setAttachError(res.error);
        }
        setOcrProgress(null);
      }
    } finally {
      setOcrBusy(false);
      setOcrProgress(null);
    }
  }, [scannedPdfs, ocrBusy]);

  const refreshModels = useCallback(async () => {
    try {
      setModels(await commands.listLocalModels());
    } catch {
      /* keep prior models */
    }
  }, []);

  const copyMessage = useCallback((idx: number, text: string) => {
    void navigator.clipboard
      .writeText(text)
      .then(() => {
        setCopiedIdx(idx);
        window.setTimeout(
          () => setCopiedIdx((c) => (c === idx ? null : c)),
          1200,
        );
      })
      .catch(() => {
        /* clipboard denied */
      });
  }, []);
  useEffect(() => {
    void refreshModels();
  }, [refreshModels]);

  // Mirror the active chat's messages into the stored conversation (upsert + move-to-top).
  useEffect(() => {
    if (!activeId || messages.length === 0) return;
    setConversations((prev) => {
      const rest = prev.filter((c) => c.id !== activeId);
      const conv: Conversation = {
        id: activeId,
        title: titleFrom(messages),
        messages,
        updatedAt: Date.now(),
      };
      return [conv, ...rest];
    });
  }, [messages, activeId]);

  // Stream listeners — each event matched to the current turn so stale/cancelled streams are ignored.
  useEffect(() => {
    const unlisten: Array<() => void> = [];
    void listen<{ id: number; text: string }>("chat-token", (e) => {
      if (e.payload.id !== turnIdRef.current) return;
      setMessages((m) => appendToLastAssistant(m, e.payload.text));
    }).then((u) => unlisten.push(u));
    void listen<{ id: number; text: string }>("chat-done", (e) => {
      if (e.payload.id !== turnIdRef.current) return;
      setMessages((m) => replaceLastAssistant(m, e.payload.text));
      setStreaming(false);
    }).then((u) => unlisten.push(u));
    void listen<{ id: number; message: string }>("chat-error", (e) => {
      if (e.payload.id !== turnIdRef.current) return;
      setMessages((m) => replaceLastAssistant(m, e.payload.message, true));
      setStreaming(false);
    }).then((u) => unlisten.push(u));
    // Big-document summarize progress: drive the spinner + label as it maps/reduces.
    void listen<{ done: number; total: number; stage: string }>(
      "doc-summarize-progress",
      (e) => {
        if (!summarizeActiveRef.current) return;
        setSummarizeProgress(e.payload);
      },
    ).then((u) => unlisten.push(u));
    // OCR progress ("Reading page 12 of 105") for a long scanned chart.
    void listen<{ done: number; total: number }>("ocr-progress", (e) => {
      setOcrProgress(e.payload);
    }).then((u) => unlisten.push(u));
    return () => unlisten.forEach((u) => u());
  }, []);

  // Keep the newest message in view as it streams.
  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages]);

  // Auto-grow the composer textarea from one row up to a cap, so it starts small and matches the slide-out.
  useEffect(() => {
    const el = taRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 140)}px`;
  }, [input]);

  const send = useCallback(
    async (presetText?: string) => {
      if (streaming) return;
      // A preset (one-tap HPI / problem-list) overrides the box; otherwise use what's typed. With a document
      // attached, an empty box means "summarize the whole thing".
      const text =
        (presetText ?? input).trim() ||
        (attachedDoc
          ? "Summarize the entire attached document — cover all of its sections and pages, not just the beginning."
          : "");
      if (!text) return;
      let id = activeId;
      if (!id) {
        id = newId();
        setActiveId(id);
      }
      const turn = turnIdRef.current + 1;
      turnIdRef.current = turn;
      const next: Msg[] = [
        ...messages,
        { role: "user", content: text },
        { role: "assistant", content: "" },
      ];
      setMessages(next);
      if (presetText === undefined) setInput(""); // a preset shouldn't wipe whatever's typed in the box
      setStreaming(true);

      // A document too large to fit the context window can't be chatted with directly — route it through the
      // offline map/reduce summarizer, which reads it in parts and never exceeds the stable 16k window. The
      // user's message becomes the instruction (what to produce, e.g. a comprehensive HPI).
      const SAFE_CTX_CAP = 16384;
      if (attachedDoc && usedTokens + 2048 > SAFE_CTX_CAP) {
        summarizeActiveRef.current = true;
        // Seed the spinner immediately; the backend's progress events refine the label as it works.
        setSummarizeProgress({
          done: 0,
          total: 0,
          stage: t("chat.summarizeReading", "Reading the document"),
        });
        const res = await commands.summarizeDocument(attachedDoc.text, text);
        summarizeActiveRef.current = false;
        setSummarizeProgress(null);
        if (turn === turnIdRef.current) {
          if (res.status === "ok") {
            setMessages((m) => replaceLastAssistant(m, res.data));
          } else {
            setMessages((m) => replaceLastAssistant(m, res.error, true));
          }
          setStreaming(false);
        }
        return;
      }

      // Inject the attached document as a context turn (kept out of the visible transcript) so it's available
      // every turn while attached — enabling follow-up questions about it.
      const docContext = attachedDoc
        ? [
            {
              role: "user",
              content: `The user attached a document titled "${attachedDoc.name}". Use it to answer their requests.\n\n<document>\n${attachedDoc.text}\n</document>`,
            },
            { role: "assistant", content: "I've read the document." },
          ]
        : [];
      const payload = [
        { role: "system", content: SYSTEM_PROMPT + reasonSuffix(reason) },
        ...docContext,
        ...next.slice(0, -1).map((m) => ({ role: m.role, content: m.content })),
      ];
      // When a document is attached it can dwarf the selected context window. Grow the context to fit the doc +
      // answer, capped at a VRAM-safe 16k (stable fp16 KV). Beyond that we routed to summarize_document above;
      // here the doc fits, so just grow the window to hold it.
      let effectiveCtx = ctxTokens;
      if (attachedDoc) {
        const needed = usedTokens + 2048; // prompt (incl. doc) + a little answer headroom
        effectiveCtx = Math.min(
          Math.max(ctxTokens, Math.ceil(needed / 2048) * 2048),
          SAFE_CTX_CAP,
        );
      }
      const res = await commands.chatStream(turn, payload, effectiveCtx);
      if (res.status === "error" && turn === turnIdRef.current) {
        setMessages((m) => replaceLastAssistant(m, res.error, true));
        setStreaming(false);
      }
    },
    [
      input,
      streaming,
      messages,
      activeId,
      ctxTokens,
      reason,
      attachedDoc,
      usedTokens,
    ],
  );

  const stop = useCallback(async () => {
    await commands.chatCancel(turnIdRef.current);
  }, []);

  const newChat = useCallback(() => {
    if (streaming) return;
    setActiveId(null);
    setMessages([]);
    setInput("");
  }, [streaming]);

  const loadConv = useCallback(
    (c: Conversation) => {
      if (streaming) return;
      setActiveId(c.id);
      setMessages(c.messages);
    },
    [streaming],
  );

  const deleteConv = useCallback(
    (id: string, e: MouseEvent) => {
      e.stopPropagation();
      setConversations((prev) => prev.filter((c) => c.id !== id));
      if (activeId === id) {
        setActiveId(null);
        setMessages([]);
      }
    },
    [activeId],
  );

  const onModelChange = useCallback(
    async (path: string) => {
      if (!path) return;
      await commands.setLocalModel(path);
      await refreshModels();
    },
    [refreshModels],
  );

  // Import any .gguf from disk and make it the active model (points at it in place; nothing is copied).
  const importModel = useCallback(async () => {
    try {
      const picked = await open({
        multiple: false,
        filters: [{ name: "GGUF model", extensions: ["gguf"] }],
      });
      if (typeof picked === "string") {
        const res = await commands.setLocalModel(picked);
        if (res.status === "ok") await refreshModels();
      }
    } catch {
      /* dialog cancelled or unavailable */
    }
  }, [refreshModels]);

  const toggleRail = useCallback(() => {
    setRailOpen((o) => {
      const next = !o;
      try {
        localStorage.setItem("dotflow.chat.railOpen", String(next));
      } catch {
        /* best-effort */
      }
      return next;
    });
  }, []);

  const activeModel = models.find((m) => m.active);

  return (
    <div className="flex h-full min-h-0 text-sm">
      {/* Conversations rail (collapsible) */}
      {railOpen && (
        <aside className="flex w-56 shrink-0 flex-col border-r border-neutral-200 dark:border-neutral-800">
          <div className="p-2">
            <button
              type="button"
              onClick={newChat}
              disabled={streaming}
              className="flex w-full items-center gap-2 rounded-lg border border-neutral-300 px-3 py-2 text-sm hover:bg-neutral-100 disabled:opacity-40 dark:border-neutral-700 dark:hover:bg-neutral-800"
            >
              <Plus size={15} /> {t("chat.newChat")}
            </button>
          </div>
          <div className="flex-1 overflow-y-auto px-2 pb-2">
            <div className="px-1 pb-1 pt-1 text-[10.5px] font-semibold uppercase tracking-wide text-neutral-400">
              {t("chat.recent")}
            </div>
            {conversations.length === 0 ? (
              <p className="px-1 py-2 text-xs text-neutral-400">
                {t("chat.noHistory")}
              </p>
            ) : (
              conversations.map((c) => (
                <div
                  key={c.id}
                  onClick={() => loadConv(c)}
                  className={`group flex cursor-pointer items-center gap-1.5 rounded-lg px-2 py-1.5 ${
                    activeId === c.id
                      ? "bg-neutral-100 dark:bg-neutral-800"
                      : "hover:bg-neutral-100/70 dark:hover:bg-neutral-800/60"
                  }`}
                >
                  <MessageSquare
                    size={13}
                    className="shrink-0 text-neutral-400"
                  />
                  <span className="flex-1 truncate text-[13px]" title={c.title}>
                    {c.title}
                  </span>
                  <button
                    type="button"
                    onClick={(e) => deleteConv(c.id, e)}
                    className="text-neutral-400 opacity-0 hover:text-red-500 group-hover:opacity-100"
                    title={t("chat.delete")}
                  >
                    <Trash2 size={13} />
                  </button>
                </div>
              ))
            )}
          </div>
        </aside>
      )}

      {/* Chat column */}
      <div className="flex min-w-0 flex-1 flex-col">
        {/* Header */}
        <div className="flex items-center justify-between gap-2 border-b border-neutral-200 px-4 py-2 dark:border-neutral-800">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <button
              type="button"
              onClick={toggleRail}
              title={t("chat.toggleHistory")}
              className="rounded-md p-1 text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800"
            >
              {railOpen ? (
                <PanelLeftClose size={16} />
              ) : (
                <PanelLeftOpen size={16} />
              )}
            </button>
            <span className="text-xs text-neutral-500">{t("chat.model")}</span>
            {models.length > 0 ? (
              <select
                className="max-w-[260px] rounded-md border border-neutral-300 bg-transparent px-2 py-1 text-sm dark:border-neutral-700"
                value={activeModel?.path ?? ""}
                onChange={(e) => void onModelChange(e.target.value)}
                disabled={streaming}
              >
                {!activeModel && (
                  <option value="">{t("chat.selectModel")}</option>
                )}
                {models.map((m) => (
                  <option key={m.path} value={m.path}>
                    {m.name}
                  </option>
                ))}
              </select>
            ) : (
              <span className="text-xs text-neutral-500">
                {t("chat.noModel")}
              </span>
            )}
            <button
              type="button"
              onClick={() => void importModel()}
              disabled={streaming}
              className="flex items-center gap-1 rounded-md border border-neutral-300 px-2 py-1 text-xs text-neutral-600 hover:bg-neutral-100 disabled:opacity-40 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800"
              title={t("chat.importModel")}
            >
              <Upload size={13} /> {t("chat.import")}
            </button>
            <span className="ml-1 text-xs text-neutral-500">
              {t("chat.context")}
            </span>
            <select
              className="rounded-md border border-neutral-300 bg-transparent px-2 py-1 text-sm dark:border-neutral-700"
              value={ctxTokens}
              onChange={(e) => {
                const v = parseInt(e.target.value, 10);
                setCtxTokens(v);
                try {
                  localStorage.setItem("dotflow.chat.ctx", String(v));
                } catch {
                  /* best-effort */
                }
              }}
              disabled={streaming}
              title={t("chat.contextHint")}
            >
              {[4096, 8192, 16384].map((n) => (
                <option key={n} value={n}>
                  {t("chat.tokensK", { count: n / 1024 })}
                </option>
              ))}
            </select>
            <span
              className={`ml-1 whitespace-nowrap text-xs tabular-nums ${
                ctxPct >= 90
                  ? "text-red-500"
                  : ctxPct >= 70
                    ? "text-amber-500"
                    : "text-neutral-500"
              }`}
              title={t(
                "chat.contextUsageHint",
                "Approximate share of the context window used by this conversation",
              )}
            >
              {t("chat.contextUsage", "≈{{used}} / {{max}}k ({{pct}}%)", {
                used: usedTokens.toLocaleString(),
                max: ctxTokens / 1024,
                pct: ctxPct,
              })}
            </span>
            <button
              type="button"
              onClick={toggleReason}
              title={t(
                "chat.reasonHint",
                "Reasoning: let the model think before answering (slower, better on complex questions)",
              )}
              className={`ml-1 flex items-center gap-1 rounded-md border px-1.5 py-1 text-xs transition-colors ${
                reason
                  ? "border-emerald-400 bg-emerald-50 text-emerald-600 dark:bg-emerald-900/20"
                  : "border-neutral-300 text-neutral-500 hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800"
              }`}
            >
              <Brain size={13} />
              {t("chat.reason", "Reason")}
            </button>
          </div>
        </div>

        {/* Messages */}
        <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto">
          {messages.length === 0 ? (
            <div className="flex h-full flex-col items-center justify-center px-6 text-center">
              <p className="text-neutral-400">{t("chat.empty")}</p>
            </div>
          ) : (
            <div className="mx-auto flex max-w-3xl flex-col gap-6 px-4 py-6">
              {messages.map((m, i) => (
                <div key={i} className="group flex flex-col gap-1">
                  <div className="flex items-center gap-2">
                    <span className="text-[11px] font-semibold uppercase tracking-wide text-neutral-400">
                      {m.role === "user"
                        ? t("chat.roleYou")
                        : t("chat.roleAssistant")}
                    </span>
                    {m.content && !m.error && (
                      <button
                        type="button"
                        onClick={() =>
                          copyMessage(
                            i,
                            m.role === "assistant"
                              ? parseThinking(sanitize(m.content)).answer ||
                                  sanitize(m.content)
                              : m.content,
                          )
                        }
                        className="text-neutral-400 opacity-0 transition hover:text-neutral-600 group-hover:opacity-100 dark:hover:text-neutral-200"
                        title={t("chat.copy")}
                      >
                        {copiedIdx === i ? (
                          <Check size={13} />
                        ) : (
                          <Copy size={13} />
                        )}
                      </button>
                    )}
                  </div>
                  {m.role === "assistant" ? (
                    (() => {
                      const { thinking, answer } = parseThinking(
                        sanitize(m.content),
                      );
                      const isLast = i === messages.length - 1;
                      return (
                        <>
                          {thinking && (
                            <details className="mb-1 rounded-lg border border-neutral-200 bg-neutral-50 px-2 py-1 text-xs text-neutral-500 dark:border-neutral-800 dark:bg-neutral-800/30">
                              <summary className="cursor-pointer select-none">
                                {t("chat.reasoning")}
                              </summary>
                              <div className="mt-1 select-text whitespace-pre-wrap leading-relaxed">
                                {thinking}
                              </div>
                            </details>
                          )}
                          {m.error ? (
                            <div className="select-text whitespace-pre-wrap leading-relaxed text-red-600">
                              {answer}
                            </div>
                          ) : answer ? (
                            <div className="select-text cursor-text leading-relaxed">
                              <ChatMarkdown>{answer}</ChatMarkdown>
                            </div>
                          ) : summarizeProgress && isLast ? (
                            <div className="flex items-center gap-2 text-neutral-500">
                              <Loader2
                                size={15}
                                className="animate-spin text-emerald-600"
                              />
                              <span>{summarizeProgress.stage}…</span>
                            </div>
                          ) : (
                            <div className="leading-relaxed text-neutral-500">
                              {streaming && isLast
                                ? thinking
                                  ? t("chat.thinking")
                                  : "…"
                                : ""}
                            </div>
                          )}
                        </>
                      );
                    })()
                  ) : (
                    <div className="select-text cursor-text whitespace-pre-wrap leading-relaxed text-neutral-800 dark:text-neutral-100">
                      {m.content}
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Composer */}
        <div className="border-t border-neutral-200 px-4 py-2 dark:border-neutral-800">
          {(docParts.length > 0 || attachError || scannedPdfs.length > 0) && (
            <div className="mx-auto mb-1.5 max-w-3xl">
              {/* Attached files — one chip per file, combined into a single record; remove per file. */}
              {docParts.length > 0 && (
                <div className="flex flex-wrap gap-1.5">
                  {docParts.map((d, i) => (
                    <div
                      key={`${d.name}-${i}`}
                      className="inline-flex items-center gap-1.5 rounded-lg border border-neutral-300 bg-neutral-100 px-2 py-1 text-xs dark:border-neutral-700 dark:bg-neutral-800"
                    >
                      <FileText size={12} className="text-emerald-600" />
                      <span className="max-w-[200px] truncate" title={d.name}>
                        {d.name}
                      </span>
                      <span className="text-neutral-400">
                        {t("chat.docChars", "· {{n}} chars", {
                          n: d.text.length.toLocaleString(),
                        })}
                      </span>
                      <button
                        type="button"
                        onClick={() =>
                          setDocParts((prev) => prev.filter((_, j) => j !== i))
                        }
                        className="text-neutral-400 hover:text-red-500"
                        title={t("chat.removeDoc", "Remove")}
                      >
                        <X size={12} />
                      </button>
                    </div>
                  ))}
                </div>
              )}
              {/* One-tap clinical presets — run against all attached files combined. */}
              {attachedDoc && (
                <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                  <span className="text-[11px] text-neutral-400">
                    {t("chat.presets", "Quick:")}
                  </span>
                  <button
                    type="button"
                    disabled={streaming}
                    onClick={() => void send(HPI_PROMPT)}
                    className="rounded-md border border-neutral-300 px-2 py-0.5 text-xs text-neutral-600 hover:bg-neutral-100 disabled:opacity-40 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800"
                  >
                    {t("chat.presetHpi", "HPI")}
                  </button>
                  <button
                    type="button"
                    disabled={streaming}
                    onClick={() => void send(PROBLEM_LIST_PROMPT)}
                    className="rounded-md border border-neutral-300 px-2 py-0.5 text-xs text-neutral-600 hover:bg-neutral-100 disabled:opacity-40 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800"
                  >
                    {t("chat.presetProblems", "Problem list + ICD-10")}
                  </button>
                  {/* User's own saved presets, each with a small delete affordance. */}
                  {customPresets.map((p) => (
                    <span key={p.id} className="group inline-flex items-center">
                      <button
                        type="button"
                        disabled={streaming}
                        onClick={() => void send(p.prompt)}
                        title={p.prompt}
                        className="rounded-l-md border border-neutral-300 px-2 py-0.5 text-xs text-neutral-600 hover:bg-neutral-100 disabled:opacity-40 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800"
                      >
                        {p.label}
                      </button>
                      <button
                        type="button"
                        onClick={() => deletePreset(p.id)}
                        title={t("chat.presetDelete", "Delete preset")}
                        className="rounded-r-md border border-l-0 border-neutral-300 px-1 py-0.5 text-neutral-400 hover:bg-neutral-100 hover:text-red-500 dark:border-neutral-700 dark:hover:bg-neutral-800"
                      >
                        <X size={11} />
                      </button>
                    </span>
                  ))}
                  <button
                    type="button"
                    onClick={() => setPresetEditorOpen((o) => !o)}
                    title={t("chat.presetAddTitle", "Save a custom preset")}
                    className="rounded-md border border-dashed border-neutral-300 px-2 py-0.5 text-xs text-neutral-500 hover:bg-neutral-100 dark:border-neutral-600 dark:hover:bg-neutral-800"
                  >
                    {t("chat.presetAdd", "+ Preset")}
                  </button>
                </div>
              )}
              {/* Inline editor to add a personal preset button. */}
              {attachedDoc && presetEditorOpen && (
                <div className="mt-1.5 flex flex-col gap-1.5 rounded-lg border border-neutral-300 bg-neutral-50 p-2 dark:border-neutral-700 dark:bg-neutral-900/40">
                  <input
                    value={newPresetLabel}
                    onChange={(e) => setNewPresetLabel(e.target.value)}
                    placeholder={t(
                      "chat.presetLabelPh",
                      "Button label (e.g. SBAR handoff)",
                    )}
                    className="rounded-md border border-neutral-300 bg-transparent px-2 py-1 text-xs outline-none focus:border-emerald-400 dark:border-neutral-700"
                  />
                  <textarea
                    value={newPresetPrompt}
                    onChange={(e) => setNewPresetPrompt(e.target.value)}
                    placeholder={t(
                      "chat.presetPromptPh",
                      "The prompt to run against the attached document(s)…",
                    )}
                    rows={3}
                    className="resize-none rounded-md border border-neutral-300 bg-transparent px-2 py-1 text-xs outline-none focus:border-emerald-400 dark:border-neutral-700"
                  />
                  <div className="flex gap-1.5">
                    <button
                      type="button"
                      onClick={savePreset}
                      disabled={
                        !newPresetLabel.trim() || !newPresetPrompt.trim()
                      }
                      className="rounded-md bg-emerald-600 px-2.5 py-1 text-xs font-medium text-white disabled:opacity-40"
                    >
                      {t("chat.presetSave", "Save preset")}
                    </button>
                    <button
                      type="button"
                      onClick={() => setPresetEditorOpen(false)}
                      className="rounded-md border border-neutral-300 px-2.5 py-1 text-xs text-neutral-600 hover:bg-neutral-100 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800"
                    >
                      {t("chat.presetCancel", "Cancel")}
                    </button>
                  </div>
                </div>
              )}
              {attachError && (
                <div className="mt-1 text-xs text-red-500">{attachError}</div>
              )}
              {scannedPdfs.length > 0 && (
                <button
                  type="button"
                  onClick={() => void runOcr()}
                  disabled={ocrBusy}
                  className="mt-1.5 inline-flex items-center gap-1.5 rounded-lg border border-emerald-400 bg-emerald-50 px-2.5 py-1 text-xs font-medium text-emerald-700 hover:bg-emerald-100 disabled:opacity-60 dark:bg-emerald-900/20 dark:text-emerald-300"
                >
                  <FileText size={12} />
                  {ocrBusy
                    ? ocrProgress && ocrProgress.total > 0
                      ? t(
                          "chat.ocrPage",
                          "Reading page {{done}} of {{total}}…",
                          {
                            done: ocrProgress.done + 1,
                            total: ocrProgress.total,
                          },
                        )
                      : t(
                          "chat.ocrRunning",
                          "Reading pages… (this can take a bit)",
                        )
                    : t("chat.ocrRunN", "Read {{n}} scanned file(s) with OCR", {
                        n: scannedPdfs.length,
                      })}
                </button>
              )}
            </div>
          )}
          <div className="mx-auto flex max-w-3xl items-end gap-1.5 rounded-xl border border-neutral-300 bg-neutral-50 px-2.5 py-1.5 shadow-sm focus-within:border-emerald-400 focus-within:ring-2 focus-within:ring-emerald-100 dark:border-neutral-700 dark:bg-neutral-900/40 dark:focus-within:ring-emerald-900/30">
            <button
              type="button"
              onClick={() => void attachPdf()}
              title={t("chat.attachPdf", "Attach a PDF")}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
            >
              <Paperclip size={15} />
            </button>
            <textarea
              ref={taRef}
              rows={1}
              value={input}
              placeholder={t("chat.placeholder")}
              className="max-h-[140px] flex-1 resize-none overflow-y-auto bg-transparent py-1 text-sm leading-normal outline-none placeholder:text-neutral-400"
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void send();
                }
              }}
            />
            <button
              type="button"
              onClick={() => void toggleMic(input)}
              className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition-colors ${
                recording
                  ? "animate-pulse bg-red-500 text-white"
                  : "text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
              }`}
              title={t("chat.voice")}
            >
              <Mic size={15} />
            </button>
            {streaming ? (
              <button
                type="button"
                onClick={() => void stop()}
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-neutral-800 text-white"
                title={t("chat.stop")}
              >
                <Square size={15} />
              </button>
            ) : (
              <button
                type="button"
                onClick={() => void send()}
                disabled={!input.trim() && !attachedDoc}
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-emerald-600 text-white disabled:opacity-40"
                title={t("chat.send")}
              >
                <Send size={15} />
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
