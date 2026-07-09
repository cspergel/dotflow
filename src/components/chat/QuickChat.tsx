import { useState, useRef, useEffect, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { Send, Square, Mic } from "lucide-react";
import { commands } from "../../bindings";
import { sanitize, parseThinking } from "./chatText";

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
  const [recording, setRecording] = useState(false);
  const turnIdRef = useRef(0);
  const scrollRef = useRef<HTMLDivElement>(null);

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
      { role: "system", content: QUICK_SYSTEM },
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
  }, [input, streaming, messages]);

  const toggleMic = useCallback(async () => {
    if (recording) {
      setRecording(false);
      const res = await commands.chatDictateStop();
      if (res.status === "ok" && res.data.trim()) {
        const tx = res.data.trim();
        setInput((p) => (p.trim() ? p.trimEnd() + " " + tx : tx));
      }
    } else {
      const res = await commands.chatDictateStart();
      if (res.status === "ok") setRecording(true);
    }
  }, [recording]);

  return (
    <div className="flex h-full flex-col bg-background">
      <div ref={scrollRef} className="flex-1 overflow-y-auto px-3 py-2 text-sm">
        {messages.length === 0 ? (
          <p className="mt-6 text-center text-xs text-neutral-400">
            {t("chat.quickPlaceholder")}
          </p>
        ) : (
          <div className="flex flex-col gap-3">
            {messages.map((m, i) => {
              const content =
                m.role === "assistant"
                  ? parseThinking(sanitize(m.content)).answer
                  : m.content;
              return (
                <div
                  key={i}
                  className={
                    m.role === "user"
                      ? "self-end max-w-[85%] rounded-xl bg-neutral-100 px-2.5 py-1.5 dark:bg-neutral-800"
                      : "max-w-[92%] select-text self-start whitespace-pre-wrap " +
                        (m.error ? "text-red-600" : "text-neutral-800 dark:text-neutral-100")
                  }
                >
                  {content ||
                    (streaming && i === messages.length - 1 ? "…" : "")}
                </div>
              );
            })}
          </div>
        )}
      </div>
      <div className="flex items-end gap-1.5 border-t border-neutral-200 px-2 py-2 dark:border-neutral-800">
        <textarea
          rows={1}
          value={input}
          placeholder={t("chat.quickPlaceholder")}
          className="max-h-24 flex-1 resize-none rounded-lg border border-neutral-300 bg-transparent px-2 py-1.5 text-sm outline-none dark:border-neutral-700"
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
          onClick={() => void toggleMic()}
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
