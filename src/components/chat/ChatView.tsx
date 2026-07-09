import { useState, useRef, useEffect, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { Send, Square, Plus } from "lucide-react";
import { commands, type LlmModelInfo } from "../../bindings";

type ChatRole = "user" | "assistant";
interface Msg {
  role: ChatRole;
  content: string;
  error?: boolean;
}

/** Append streamed text to the trailing assistant message (the placeholder created on send). */
function appendToLastAssistant(msgs: Msg[], text: string): Msg[] {
  if (msgs.length === 0) return msgs;
  const last = msgs[msgs.length - 1];
  if (last.role !== "assistant") return msgs;
  return [...msgs.slice(0, -1), { ...last, content: last.content + text }];
}

/** Replace the trailing assistant message with the authoritative final text (chat-done) or an error. */
function replaceLastAssistant(msgs: Msg[], text: string, error = false): Msg[] {
  if (msgs.length === 0) return msgs;
  const last = msgs[msgs.length - 1];
  if (last.role !== "assistant") return msgs;
  return [...msgs.slice(0, -1), { role: "assistant", content: text, error }];
}

/**
 * Offline AI chat, backed by the local GGUF model. Streams tokens live via the `chat-token` / `chat-done` /
 * `chat-error` events emitted by `commands::chat`. Reused in the sidebar section and (later) the condensed
 * slide-out.
 */
export default function ChatView() {
  const { t } = useTranslation();
  const [messages, setMessages] = useState<Msg[]>([]);
  const [input, setInput] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [models, setModels] = useState<LlmModelInfo[]>([]);
  const turnIdRef = useRef(0);
  const scrollRef = useRef<HTMLDivElement>(null);

  const refreshModels = useCallback(async () => {
    try {
      setModels(await commands.listLlmModels());
    } catch {
      /* leave models as-is */
    }
  }, []);

  useEffect(() => {
    void refreshModels();
  }, [refreshModels]);

  // Persistent stream listeners; each event is matched to the current turn so stale/cancelled streams are
  // ignored. Registered once.
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

  const send = useCallback(async () => {
    const text = input.trim();
    if (!text || streaming) return;
    const id = turnIdRef.current + 1;
    turnIdRef.current = id;
    const next: Msg[] = [
      ...messages,
      { role: "user", content: text },
      { role: "assistant", content: "" },
    ];
    setMessages(next);
    setInput("");
    setStreaming(true);
    // Send the conversation WITHOUT the empty trailing assistant placeholder.
    const payload = next
      .slice(0, -1)
      .map((m) => ({ role: m.role, content: m.content }));
    const res = await commands.chatStream(id, payload);
    if (res.status === "error" && id === turnIdRef.current) {
      // Pre-generation failures (no model, missing file) don't emit chat-error, so surface them here.
      setMessages((m) => replaceLastAssistant(m, res.error, true));
      setStreaming(false);
    }
  }, [input, streaming, messages]);

  const stop = useCallback(async () => {
    await commands.chatCancel(turnIdRef.current);
  }, []);

  const newChat = useCallback(() => {
    if (!streaming) setMessages([]);
  }, [streaming]);

  const onModelChange = useCallback(
    async (id: string) => {
      await commands.selectLlmModel(id);
      await refreshModels();
    },
    [refreshModels],
  );

  const downloaded = models.filter((m) => m.downloaded);
  const activeModel = models.find((m) => m.active);

  return (
    <div className="flex h-full flex-col gap-3">
      {/* Header: model dropdown + new chat */}
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="text-sm text-neutral-500">{t("chat.model")}</span>
          {downloaded.length > 0 ? (
            <select
              className="rounded-md border border-neutral-300 bg-transparent px-2 py-1 text-sm"
              value={activeModel?.id ?? ""}
              onChange={(e) => void onModelChange(e.target.value)}
              disabled={streaming}
            >
              {downloaded.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                </option>
              ))}
            </select>
          ) : (
            <span className="text-sm text-neutral-500">{t("chat.noModel")}</span>
          )}
        </div>
        <button
          type="button"
          onClick={newChat}
          disabled={streaming || messages.length === 0}
          className="flex items-center gap-1 rounded-md border border-neutral-300 px-2 py-1 text-sm disabled:opacity-40"
          title={t("chat.newChat")}
        >
          <Plus size={14} />
          {t("chat.newChat")}
        </button>
      </div>

      {/* Messages */}
      <div
        ref={scrollRef}
        className="flex-1 overflow-y-auto rounded-lg border border-neutral-200 p-3"
      >
        {messages.length === 0 ? (
          <p className="mt-8 text-center text-sm text-neutral-400">
            {t("chat.empty")}
          </p>
        ) : (
          <div className="flex flex-col gap-3">
            {messages.map((m, i) => (
              <div
                key={i}
                className={
                  m.role === "user"
                    ? "self-end max-w-[85%] rounded-2xl bg-neutral-100 px-3 py-2 text-sm whitespace-pre-wrap"
                    : "self-start max-w-[85%] rounded-2xl px-3 py-2 text-sm whitespace-pre-wrap " +
                      (m.error ? "text-red-600" : "text-neutral-800")
                }
              >
                {m.content ||
                  (streaming && i === messages.length - 1 ? "…" : "")}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Input */}
      <div className="flex items-end gap-2">
        <textarea
          className="min-h-[44px] max-h-40 flex-1 resize-none rounded-lg border border-neutral-300 bg-transparent px-3 py-2 text-sm"
          rows={1}
          value={input}
          placeholder={t("chat.placeholder")}
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
            className="flex h-[44px] items-center gap-1 rounded-lg bg-neutral-800 px-3 text-sm text-white"
            title={t("chat.stop")}
          >
            <Square size={14} />
            {t("chat.stop")}
          </button>
        ) : (
          <button
            type="button"
            onClick={() => void send()}
            disabled={!input.trim()}
            className="flex h-[44px] items-center gap-1 rounded-lg bg-emerald-600 px-3 text-sm text-white disabled:opacity-40"
            title={t("chat.send")}
          >
            <Send size={14} />
            {t("chat.send")}
          </button>
        )}
      </div>
    </div>
  );
}
