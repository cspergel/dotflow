import { useState, useRef, useEffect, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import {
  Send,
  Square,
  Mic,
  History,
  Plus,
  MessageSquare,
  Brain,
} from "lucide-react";
import { commands } from "../../bindings";
import { sanitize, parseThinking } from "./chatText";
import { ChatMarkdown } from "./ChatMarkdown";
import { useChatDictation } from "./useChatDictation";
import {
  upsertConversation,
  loadConversations,
  titleFrom,
  newId,
  loadReason,
  saveReason,
  reasonSuffix,
  QUICK_CONV_KEY,
  type Conversation,
} from "./chatStore";

const QUICK_SYSTEM =
  "You are DotFlow, a helpful offline assistant running privately on the user's computer. Answer quick " +
  "questions concisely and directly; give general information and assistance without refusing.";

type Role = "user" | "assistant";
interface Msg {
  role: Role;
  content: string;
  error?: boolean;
}

/**
 * Compact chat for the condensed (bar/mini) view's slide-out. Ephemeral (no history/model-dropdown) — reuses
 * the same streaming backend as the full ChatView with the currently-selected model. Rendered only in the
 * compact view, never alongside ChatView, so they don't share event streams.
 */
export default function QuickChat() {
  const { t } = useTranslation();
  const [messages, setMessages] = useState<Msg[]>([]);
  const [input, setInput] = useState("");
  const [streaming, setStreaming] = useState(false);
  const { recording, toggleMic } = useChatDictation(setInput);
  const [showHistory, setShowHistory] = useState(false);
  const [recent, setRecent] = useState<Conversation[]>([]);
  const [reason, setReason] = useState<boolean>(loadReason);
  const turnIdRef = useRef(0);
  const convIdRef = useRef<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);

  // Auto-grow the composer as you type (capped), so long questions stay fully visible; scrolls past the cap.
  useEffect(() => {
    const el = taRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 140)}px`;
  }, [input]);

  // Open/close the recent-chats list (refreshes from the shared store each time it opens).
  const toggleHistory = useCallback(() => {
    setShowHistory((v) => {
      if (!v) setRecent(loadConversations());
      return !v;
    });
  }, []);

  // Load an old conversation into the slide-out for quick reference / continuation.
  const loadOld = useCallback((c: Conversation) => {
    setMessages(c.messages);
    convIdRef.current = c.id;
    try {
      localStorage.setItem(QUICK_CONV_KEY, c.id);
    } catch {
      /* best-effort */
    }
    setShowHistory(false);
  }, []);

  // Start a fresh quick conversation.
  const newQuick = useCallback(() => {
    if (streaming) return;
    setMessages([]);
    convIdRef.current = null;
    setShowHistory(false);
  }, [streaming]);

  // Mirror this quick conversation into the shared store so "expand" can continue it in the full window.
  useEffect(() => {
    if (!convIdRef.current || messages.length === 0) return;
    upsertConversation({
      id: convIdRef.current,
      title: titleFrom(messages),
      messages,
      updatedAt: Date.now(),
    });
  }, [messages]);

  useEffect(() => {
    const un: Array<() => void> = [];
    void listen<{ id: number; text: string }>("chat-token", (e) => {
      if (e.payload.id !== turnIdRef.current) return;
      setMessages((m) => {
        const last = m[m.length - 1];
        if (!last || last.role !== "assistant") return m;
        return [...m.slice(0, -1), { ...last, content: last.content + e.payload.text }];
      });
    }).then((u) => un.push(u));
    void listen<{ id: number; text: string }>("chat-done", (e) => {
      if (e.payload.id !== turnIdRef.current) return;
      setMessages((m) => {
        const last = m[m.length - 1];
        if (!last || last.role !== "assistant") return m;
        return [...m.slice(0, -1), { role: "assistant", content: e.payload.text }];
      });
      setStreaming(false);
    }).then((u) => un.push(u));
    void listen<{ id: number; message: string }>("chat-error", (e) => {
      if (e.payload.id !== turnIdRef.current) return;
      setMessages((m) => {
        const last = m[m.length - 1];
        if (!last || last.role !== "assistant") return m;
        return [...m.slice(0, -1), { role: "assistant", content: e.payload.message, error: true }];
      });
      setStreaming(false);
    }).then((u) => un.push(u));
    return () => un.forEach((u) => u());
  }, []);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages]);

  const send = useCallback(async () => {
    const text = input.trim();
    if (!text || streaming) return;
    if (!convIdRef.current) {
      convIdRef.current = newId();
      try {
        localStorage.setItem(QUICK_CONV_KEY, convIdRef.current);
      } catch {
        /* best-effort handoff pointer */
      }
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
      { role: "system", content: QUICK_SYSTEM + reasonSuffix(reason) },
      ...next.slice(0, -1).map((m) => ({ role: m.role, content: m.content })),
    ];
    const res = await commands.chatStream(turn, payload, 8192);
    if (res.status === "error" && turn === turnIdRef.current) {
      setMessages((m) => {
        const last = m[m.length - 1];
        if (!last || last.role !== "assistant") return m;
        return [...m.slice(0, -1), { role: "assistant", content: res.error, error: true }];
      });
      setStreaming(false);
    }
  }, [input, streaming, messages, reason]);

  return (
    <div className="relative flex h-full flex-col bg-background">
      {/* Slim header: recent-chats toggle + new chat */}
      <div className="flex items-center gap-1 border-b border-neutral-200 px-2 py-1 dark:border-neutral-800">
        <button
          type="button"
          onClick={toggleHistory}
          title={t("chat.recent")}
          className={`flex h-6 w-6 items-center justify-center rounded-md ${
            showHistory
              ? "bg-neutral-200 text-neutral-800 dark:bg-neutral-700 dark:text-neutral-100"
              : "text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
          }`}
        >
          <History size={13} />
        </button>
        <span className="flex-1 truncate text-center text-[11px] text-neutral-400">
          {t("chat.quickTitle")}
        </span>
        <button
          type="button"
          onClick={() => {
            setReason((r) => {
              const n = !r;
              saveReason(n);
              return n;
            });
          }}
          title={t("chat.reasonHint", "Reasoning")}
          className={`flex h-6 w-6 items-center justify-center rounded-md ${
            reason
              ? "bg-emerald-100 text-emerald-600 dark:bg-emerald-900/30"
              : "text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
          }`}
        >
          <Brain size={13} />
        </button>
        <button
          type="button"
          onClick={newQuick}
          title={t("chat.newChat")}
          className="flex h-6 w-6 items-center justify-center rounded-md text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
        >
          <Plus size={14} />
        </button>
      </div>

      {/* Recent-chats dropdown (overlays the message area) */}
      {showHistory && (
        <div className="absolute inset-x-0 top-8 z-10 max-h-56 overflow-y-auto border-b border-neutral-200 bg-background px-1.5 py-1.5 shadow-md dark:border-neutral-800">
          {recent.length === 0 ? (
            <p className="px-1 py-2 text-center text-xs text-neutral-400">
              {t("chat.noHistory")}
            </p>
          ) : (
            recent.map((c) => (
              <button
                key={c.id}
                type="button"
                onClick={() => loadOld(c)}
                className="flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-left hover:bg-neutral-100 dark:hover:bg-neutral-800"
              >
                <MessageSquare size={12} className="shrink-0 text-neutral-400" />
                <span className="flex-1 truncate text-[12px]" title={c.title}>
                  {c.title}
                </span>
              </button>
            ))
          )}
        </div>
      )}

      <div ref={scrollRef} className="flex-1 overflow-y-auto px-3 py-2 text-sm">
        {messages.length === 0 ? (
          <p className="mt-6 text-center text-xs text-neutral-400">
            {t("chat.quickPlaceholder")}
          </p>
        ) : (
          <div className="flex flex-col gap-3">
            {messages.map((m, i) => {
              if (m.role === "user") {
                return (
                  <div
                    key={i}
                    className="max-w-[85%] self-end rounded-xl bg-neutral-100 px-2.5 py-1.5 whitespace-pre-wrap dark:bg-neutral-800"
                  >
                    {m.content}
                  </div>
                );
              }
              const answer = parseThinking(sanitize(m.content)).answer;
              const placeholder =
                streaming && i === messages.length - 1 ? "…" : "";
              return (
                <div key={i} className="max-w-[92%] select-text self-start">
                  {m.error ? (
                    <span className="whitespace-pre-wrap text-red-600">
                      {answer || placeholder}
                    </span>
                  ) : answer ? (
                    <ChatMarkdown>{answer}</ChatMarkdown>
                  ) : (
                    <span className="text-neutral-400">{placeholder}</span>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
      <div className="flex items-end gap-1.5 border-t border-neutral-200 px-2 py-2 dark:border-neutral-800">
        <textarea
          ref={taRef}
          rows={1}
          value={input}
          placeholder={t("chat.quickPlaceholder")}
          className="max-h-[140px] flex-1 resize-none overflow-y-auto rounded-lg border border-neutral-300 bg-transparent px-2 py-1.5 text-sm outline-none dark:border-neutral-700"
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
          title={t("chat.voice")}
          className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg ${
            recording
              ? "animate-pulse bg-red-500 text-white"
              : "text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
          }`}
        >
          <Mic size={14} />
        </button>
        {streaming ? (
          <button
            type="button"
            onClick={() => void commands.chatCancel(turnIdRef.current)}
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-neutral-800 text-white"
            title={t("chat.stop")}
          >
            <Square size={14} />
          </button>
        ) : (
          <button
            type="button"
            onClick={() => void send()}
            disabled={!input.trim()}
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-emerald-600 text-white disabled:opacity-40"
            title={t("chat.send")}
          >
            <Send size={14} />
          </button>
        )}
      </div>
    </div>
  );
}
