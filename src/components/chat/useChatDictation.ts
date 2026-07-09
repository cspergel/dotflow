import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands } from "../../bindings";

function joinBase(base: string, add: string): string {
  const b = base.replace(/\s+$/, "");
  if (!add) return base;
  return b ? b + " " + add : add;
}

/**
 * Shared mic dictation for the chat composers (full ChatView + compact QuickChat). Records via the
 * chat-dictate commands and, when the STT model supports streaming, shows the transcript growing live in the
 * composer via the global `stream-text-event`. On stop, the returned transcript is authoritative and replaces
 * the live preview. Field injection is suppressed backend-side during chat dictation, so nothing types into
 * the app — the text only lands in the box. `setInput` is the composer's state setter (updater form).
 */
export function useChatDictation(
  setInput: (updater: (prev: string) => string) => void,
) {
  const [recording, setRecording] = useState(false);
  const recordingRef = useRef(false);
  // The composer text captured when recording began; dictation output is appended to it.
  const baseRef = useRef("");

  // Live preview: while recording, mirror the streaming committed+tentative text into the box.
  useEffect(() => {
    let un: (() => void) | undefined;
    void listen<{ committed: string; tentative: string }>(
      "stream-text-event",
      (e) => {
        if (!recordingRef.current) return;
        const preview = [e.payload.committed, e.payload.tentative]
          .map((s) => s.trim())
          .filter(Boolean)
          .join(" ");
        setInput(() => joinBase(baseRef.current, preview));
      },
    ).then((u) => (un = u));
    return () => un?.();
  }, [setInput]);

  const toggleMic = useCallback(
    async (currentInput: string) => {
      if (recordingRef.current) {
        recordingRef.current = false;
        setRecording(false);
        const res = await commands.chatDictateStop();
        const final = res.status === "ok" ? res.data.trim() : "";
        setInput(() => joinBase(baseRef.current, final));
      } else {
        baseRef.current = currentInput;
        const res = await commands.chatDictateStart();
        if (res.status === "ok") {
          recordingRef.current = true;
          setRecording(true);
        }
        // else: another dictation is active or no mic — ignore.
      }
    },
    [setInput],
  );

  return { recording, toggleMic };
}
