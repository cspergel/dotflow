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
export default function ChatView() {
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
    // Cap at 32k — with the q8 KV cache + flash attention the backend uses, a 32k context for a 9B model
    // fits comfortably in 16 GB VRAM.
    return Number.isFinite(v) && v >= 512 ? Math.min(v, 32768) : 8192;
  });
  const [reason, setReason] = useState<boolean>(loadReason);
  const toggleReason = useCallback(() => {
    setReason((r) => {
      const next = !r;
      saveReason(next);
      return next;
    });
  }, []);
  // Attached PDF (text-extracted). Held as context and prepended to each send while attached.
  const [attachedDoc, setAttachedDoc] = useState<{
    name: string;
    text: string;
  } | null>(null);
  const [attachError, setAttachError] = useState<string | null>(null);
  // A scanned PDF (no text layer) offered for OCR: {path, name}. Set when read_pdf_text reports "scanned".
  const [scannedPdf, setScannedPdf] = useState<{
    path: string;
    name: string;
  } | null>(null);
  const [ocrBusy, setOcrBusy] = useState(false);
  const turnIdRef = useRef(0);
  const scrollRef = useRef<HTMLDivElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);

  // Persist the conversation list whenever it changes.
  useEffect(() => {
    saveConversations(conversations);
  }, [conversations]);

  // Slide-out → expand handoff: if the compact quick chat asked to continue here, open that conversation once.
  useEffect(() => {
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
  }, []);

  // Live, approximate context-window usage (system prompt + transcript + attached doc + what's being typed).
  const usedTokens = useMemo(() => {
    let sum = estimateTokens(SYSTEM_PROMPT) + estimateTokens(input);
    for (const m of messages) sum += estimateTokens(m.content);
    if (attachedDoc) sum += estimateTokens(attachedDoc.text);
    return sum;
  }, [messages, input, attachedDoc]);
  const ctxPct = Math.min(100, Math.round((usedTokens / ctxTokens) * 100));

  // Attach a PDF: pick it, extract its text locally, hold it as context for the conversation.
  const attachPdf = useCallback(async () => {
    try {
      const picked = await open({
        multiple: false,
        filters: [
          { name: "PDF", extensions: ["pdf", "PDF"] },
          { name: "All files", extensions: ["*"] },
        ],
      });
      if (typeof picked !== "string") return;
      setAttachError(null);
      setScannedPdf(null);
      const name = picked.split(/[\\/]/).pop() || "document.pdf";
      const res = await commands.readPdfText(picked);
      if (res.status === "ok") {
        setAttachedDoc({ name, text: res.data });
      } else {
        setAttachError(res.error);
        // Scanned/image PDF → offer to OCR it instead.
        if (/scanned/i.test(res.error)) setScannedPdf({ path: picked, name });
      }
    } catch {
      /* dialog cancelled */
    }
  }, []);

  // Run OCR on a scanned PDF (rasterize + read text), then attach the recognized text as the document.
  const runOcr = useCallback(async () => {
    if (!scannedPdf || ocrBusy) return;
    setOcrBusy(true);
    try {
      const res = await commands.ocrPdf(scannedPdf.path);
      if (res.status === "ok") {
        setAttachedDoc({ name: scannedPdf.name, text: res.data });
        setScannedPdf(null);
        setAttachError(null);
      } else {
        setAttachError(res.error);
      }
    } finally {
      setOcrBusy(false);
    }
  }, [scannedPdf, ocrBusy]);

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

  const send = useCallback(async () => {
    if (streaming) return;
    // With a document attached, an empty box means "summarize the whole thing".
    const text =
      input.trim() ||
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
    setInput("");
    setStreaming(true);
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
    // answer, capped at a VRAM-safe size. With the q8 KV cache + flash attention the backend uses, 32k fits a
    // 9B model in 16 GB. Beyond that the backend returns a graceful "doesn't fit" error (never a crash).
    const SAFE_CTX_CAP = 32768;
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
  }, [
    input,
    streaming,
    messages,
    activeId,
    ctxTokens,
    reason,
    attachedDoc,
    usedTokens,
  ]);

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
              {[4096, 8192, 16384, 32768].map((n) => (
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
          {(attachedDoc || attachError || scannedPdf) && (
            <div className="mx-auto mb-1.5 max-w-3xl">
              {attachedDoc && (
                <div className="inline-flex items-center gap-1.5 rounded-lg border border-neutral-300 bg-neutral-100 px-2 py-1 text-xs dark:border-neutral-700 dark:bg-neutral-800">
                  <FileText size={12} className="text-emerald-600" />
                  <span
                    className="max-w-[220px] truncate"
                    title={attachedDoc.name}
                  >
                    {attachedDoc.name}
                  </span>
                  <span className="text-neutral-400">
                    {t("chat.docChars", "· {{n}} chars", {
                      n: attachedDoc.text.length.toLocaleString(),
                    })}
                  </span>
                  <button
                    type="button"
                    onClick={() => setAttachedDoc(null)}
                    className="text-neutral-400 hover:text-red-500"
                    title={t("chat.removeDoc", "Remove")}
                  >
                    <X size={12} />
                  </button>
                </div>
              )}
              {attachError && (
                <div className="mt-1 text-xs text-red-500">{attachError}</div>
              )}
              {scannedPdf && (
                <button
                  type="button"
                  onClick={() => void runOcr()}
                  disabled={ocrBusy}
                  className="mt-1.5 inline-flex items-center gap-1.5 rounded-lg border border-emerald-400 bg-emerald-50 px-2.5 py-1 text-xs font-medium text-emerald-700 hover:bg-emerald-100 disabled:opacity-60 dark:bg-emerald-900/20 dark:text-emerald-300"
                >
                  <FileText size={12} />
                  {ocrBusy
                    ? t("chat.ocrRunning", "Reading pages… (this can take a bit)")
                    : t("chat.ocrRun", "Read it with OCR")}
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
