// Shared conversation store for the chat UIs. Both the full ChatView and the compact QuickChat slide-out
// read/write the SAME localStorage-backed list, so a quick conversation can be continued in the big window.

export type ChatRole = "user" | "assistant";
export interface ChatMsg {
  role: ChatRole;
  content: string;
  error?: boolean;
}
export interface Conversation {
  id: string;
  title: string;
  messages: ChatMsg[];
  updatedAt: number;
}

export const STORAGE_KEY = "dotflow.chat.conversations.v1";
// Handoff pointer: when set, ChatView opens this conversation on mount (used by the slide-out → expand flow).
export const OPEN_KEY = "dotflow.chat.openId";
// The slide-out records its current conversation id here so expand-to-full knows what to open.
export const QUICK_CONV_KEY = "dotflow.chat.quickConvId";

export function loadConversations(): Conversation[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

export function saveConversations(convs: Conversation[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(convs.slice(0, 50)));
  } catch {
    /* storage full / disabled — history is best-effort */
  }
}

// Upsert one conversation (move-to-top) into the persisted list. Used by the slide-out so its messages are
// available to the full window without going through React state.
export function upsertConversation(conv: Conversation) {
  const rest = loadConversations().filter((c) => c.id !== conv.id);
  saveConversations([conv, ...rest]);
}

export function titleFrom(messages: ChatMsg[]): string {
  const first = messages.find((m) => m.role === "user")?.content ?? "";
  const clean = first.trim().replace(/\s+/g, " ");
  if (!clean) return "…";
  return clean.length > 42 ? clean.slice(0, 42) + "…" : clean;
}

export function newId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `c-${Date.now()}-${Math.floor(Math.random() * 1e6)}`;
  }
}

// Rough client-side token estimate (no tokenizer in the webview). ~4 chars/token for English is close enough
// for a "how full is my context" gauge; whitespace-run collapse keeps padded text from over-counting.
export function estimateTokens(text: string): number {
  if (!text) return 0;
  const chars = text.replace(/\s+/g, " ").trim().length;
  return Math.ceil(chars / 4);
}
