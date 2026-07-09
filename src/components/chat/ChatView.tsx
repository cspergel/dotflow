import {
  useState,
  useRef,
  useEffect,
  useCallback,
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
} from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { commands, type LocalModelInfo } from "../../bindings";

// A light default persona so the local model is a genuinely helpful assistant rather than over-refusing.
// It is prepended to the request only — never shown in the transcript or stored in history.
const SYSTEM_PROMPT =
  "You are DotFlow, a helpful, knowledgeable assistant running locally and privately on the user's own " +
  "computer. Give clear, direct, useful help. Provide general educational information on any topic — " +
  "including health, medical, legal, and financial subjects — to help the user understand, plan, and draft. " +
  "You are not a substitute for a licensed professional; briefly suggest consulting one for personal " +
  "decisions, but do NOT refuse to give general information or assistance.";

type ChatRole = "user" | "assistant";
interface Msg {
  role: ChatRole;
  content: string;
  error?: boolean;
}
interface Conversation {
  id: string;
  title: string;
  messages: Msg[];
  updatedAt: number;
}

const STORAGE_KEY = "dotflow.chat.conversations.v1";

function loadConversations(): Conversation[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}
function saveConversations(convs: Conversation[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(convs.slice(0, 50)));
  } catch {
    /* storage full / disabled — history is best-effort */
  }
}
function titleFrom(messages: Msg[]): string {
  const first = messages.find((m) => m.role === "user")?.content ?? "";
  const clean = first.trim().replace(/\s+/g, " ");
  if (!clean) return "…";
  return clean.length > 42 ? clean.slice(0, 42) + "…" : clean;
}
function newId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `c-${Date.now()}-${Math.floor(Math.random() * 1e6)}`;
  }
}

// Some models leak a trailing chat-template marker into their reply (e.g. Gemma emitting `<|im_end|>` /
// `|im_end|>`, or `<end_of_turn>`) when the tokenizer doesn't treat it as a stop token. Cut the text at the
// first such marker for display/copy. (The backend also cleans this, but robustly here covers partial
// variants like a `|im_end|>` whose leading `<` tokenized separately.)
function sanitize(text: string): string {
  let out = text;
  for (const marker of [
    "<|im_end|>",
    "|im_end|>",
    "<|im_start|>",
    "<end_of_turn>",
    "<start_of_turn>",
    "<eos>",
    "</s>",
    "<|endoftext|>",
  ]) {
    const idx = out.indexOf(marker);
    if (idx !== -1) out = out.slice(0, idx);
  }
  return out;
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
  const [ctxTokens, setCtxTokens] = useState<number>(() => {
    const v = parseInt(localStorage.getItem("dotflow.chat.ctx") ?? "8192", 10);
    return Number.isFinite(v) && v >= 512 ? v : 8192;
  });
  const turnIdRef = useRef(0);
  const scrollRef = useRef<HTMLDivElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);

  // Persist the conversation list whenever it changes.
  useEffect(() => {
    saveConversations(conversations);
  }, [conversations]);

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

  // Auto-grow the composer textarea.
  useEffect(() => {
    const el = taRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, [input]);

  const send = useCallback(async () => {
    const text = input.trim();
    if (!text || streaming) return;
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
    const payload = [
      { role: "system", content: SYSTEM_PROMPT },
      ...next.slice(0, -1).map((m) => ({ role: m.role, content: m.content })),
    ];
    const res = await commands.chatStream(turn, payload, ctxTokens);
    if (res.status === "error" && turn === turnIdRef.current) {
      setMessages((m) => replaceLastAssistant(m, res.error, true));
      setStreaming(false);
    }
  }, [input, streaming, messages, activeId, ctxTokens]);

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
          <div className="flex items-center gap-2">
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
              {[4096, 8192, 16384, 32768, 65536, 131072].map((n) => (
                <option key={n} value={n}>
                  {t("chat.tokensK", { count: n / 1024 })}
                </option>
              ))}
            </select>
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
                              ? sanitize(m.content)
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
                  <div
                    className={`select-text cursor-text whitespace-pre-wrap leading-relaxed ${
                      m.error
                        ? "text-red-600"
                        : "text-neutral-800 dark:text-neutral-100"
                    }`}
                  >
                    {(m.role === "assistant"
                      ? sanitize(m.content)
                      : m.content) ||
                      (streaming && i === messages.length - 1 ? "…" : "")}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Composer */}
        <div className="border-t border-neutral-200 px-4 py-3 dark:border-neutral-800">
          <div className="mx-auto flex max-w-3xl items-end gap-2 rounded-2xl border border-neutral-300 bg-neutral-50 px-3 py-2 shadow-sm focus-within:border-emerald-400 focus-within:ring-2 focus-within:ring-emerald-100 dark:border-neutral-700 dark:bg-neutral-900/40 dark:focus-within:ring-emerald-900/30">
            <textarea
              ref={taRef}
              rows={1}
              value={input}
              placeholder={t("chat.placeholder")}
              className="max-h-[200px] flex-1 resize-none bg-transparent py-1.5 text-[15px] leading-relaxed outline-none placeholder:text-neutral-400"
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void send();
                }
              }}
            />
            {streaming ? (
              <button
                type="button"
                onClick={() => void stop()}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-neutral-800 text-white"
                title={t("chat.stop")}
              >
                <Square size={15} />
              </button>
            ) : (
              <button
                type="button"
                onClick={() => void send()}
                disabled={!input.trim()}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-emerald-600 text-white disabled:opacity-40"
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
