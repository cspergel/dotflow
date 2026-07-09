// Shared text helpers for the chat UIs (full ChatView + the compact QuickChat slide-out).

// Some models leak a trailing chat-template marker into their reply (e.g. Gemma emitting `<|im_end|>` /
// `|im_end|>`, or `<end_of_turn>`) when the tokenizer doesn't treat it as a stop token. Cut the text at the
// first such marker for display/copy. (The backend also cleans this; this covers partial variants like a
// `|im_end|>` whose leading `<` tokenized separately.)
export function sanitize(text: string): string {
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

// Reasoning models (Qwen3.x / Qwythos, DeepSeek-R1, …) emit their chain-of-thought in `<think>…</think>`
// before the answer. Split it out so the UI can hide the reasoning by default and show just the answer.
// Handles the mid-stream case where `<think>` is open but not yet closed.
export function parseThinking(text: string): {
  thinking: string | null;
  answer: string;
} {
  const start = text.indexOf("<think>");
  if (start === -1) return { thinking: null, answer: text };
  const end = text.indexOf("</think>");
  if (end === -1) {
    return { thinking: text.slice(start + 7).trim(), answer: "" };
  }
  const thinking = text.slice(start + 7, end).trim();
  const answer = (text.slice(0, start) + text.slice(end + 8)).trim();
  return { thinking, answer };
}
