/**
 * Selection-review overlay card.
 *
 * A focusable, draggable card the review hotkey pops near the cursor. It shows the offline Proofread
 * review (Harper, via ReviewPanel) plus AI action chips (Rewrite / Formal / Summarize) that stay disabled
 * until AI is configured — their real behaviour lands in Phase B. Apply pastes the reviewed result back
 * into the source field; Copy puts it on the clipboard; Close cancels; clicking away dismisses it. The
 * window sizes itself to the card's content and is not forced always-on-top.
 */
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { ReviewPanel } from "@/components/settings/cleanup/ReviewPanel";

export type ReviewAction = "proofread" | "rewrite" | "formal" | "summarize";

const CARD_WIDTH = 420;
const CARD_MAX_HEIGHT = 480;
const CARD_MIN_HEIGHT = 96;

/**
 * Pure chip-gating predicate: Proofread is offline and always available; the AI actions
 * (rewrite/formal/summarize) require a configured AI backend. Exported so the gating is trivially
 * inspectable (exercise-verified in Task A11 — the repo has no JS test runner).
 */
export function chipEnabled(
  action: ReviewAction,
  aiAvailable: boolean,
): boolean {
  return action === "proofread" ? true : aiAvailable;
}

const AI_ACTIONS: ReviewAction[] = ["rewrite", "formal", "summarize"];

const ReviewOverlay: React.FC = () => {
  const { t } = useTranslation();
  const [text, setText] = useState("");
  const [aiAvailable, setAiAvailable] = useState(false);
  const [activeAction, setActiveAction] = useState<ReviewAction>("proofread");
  const [reviewResult, setReviewResult] = useState("");
  const cardRef = useRef<HTMLDivElement>(null);
  // Timestamp until which a focus-loss must NOT close the card. Starting a native window drag
  // (data-tauri-drag-region → startDragging) briefly blurs the window; without this grace window the
  // click-away-dismiss effect would fire and close the card the instant you grab it to move it.
  const suppressCloseUntilRef = useRef(0);

  // Listeners + pull-on-mount [F11]. The backend emits "review-text" immediately after show(), which can
  // race this effect's listener registration, so we also PULL the stored payload via getPendingReview();
  // whichever arrives first wins (setting the same text twice is harmless).
  useEffect(() => {
    let cancelled = false;
    let unlistenText: (() => void) | undefined;
    let unlistenHide: (() => void) | undefined;

    const applyPayload = (nextText: string, ai: boolean) => {
      setText(nextText);
      setAiAvailable(ai);
      setActiveAction("proofread");
      setReviewResult("");
    };

    const setup = async () => {
      unlistenText = await listen("review-text", (event) => {
        const payload = event.payload as {
          text: string;
          ai_available: boolean;
        };
        applyPayload(payload.text, payload.ai_available);
      });
      unlistenHide = await listen("review-hide", () => {
        setText("");
        setReviewResult("");
        setActiveAction("proofread");
      });

      if (cancelled) {
        unlistenText?.();
        unlistenHide?.();
        return;
      }

      const pending = await commands.getPendingReview();
      if (pending && !cancelled) {
        applyPayload(pending[0], pending[1]);
      }
    };

    void setup();

    return () => {
      cancelled = true;
      unlistenText?.();
      unlistenHide?.();
    };
  }, []);

  const handleApply = useCallback(async () => {
    await commands.applyReviewResult(reviewResult || text);
  }, [reviewResult, text]);

  const handleCopy = useCallback(async () => {
    await navigator.clipboard.writeText(reviewResult || text);
  }, [reviewResult, text]);

  const handleClose = useCallback(async () => {
    await commands.cancelReview();
  }, []);

  // Window-level keyboard: Enter → Apply, Escape → Close. Enter is ignored while a button inside the card
  // is focused so it can't hijack ReviewPanel's accept-a-suggestion buttons.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Enter") {
        if ((event.target as HTMLElement)?.tagName === "BUTTON") return;
        event.preventDefault();
        void handleApply();
      } else if (event.key === "Escape") {
        event.preventDefault();
        void handleClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [handleApply, handleClose]);

  // Size the window to the card's content so it doesn't cover more of the screen than it needs (a fixed
  // 420x340 box left a lot of dead space and obscured text). Capped at CARD_MAX_HEIGHT; the body scrolls
  // beyond that. Runs on every content change via ResizeObserver.
  useEffect(() => {
    const el = cardRef.current;
    if (!el) return;
    const win = getCurrentWebviewWindow();
    const fit = () => {
      const h = Math.min(
        Math.max(Math.ceil(el.scrollHeight), CARD_MIN_HEIGHT),
        CARD_MAX_HEIGHT,
      );
      void win.setSize(new LogicalSize(CARD_WIDTH, h)).catch(() => {});
    };
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    fit();
    return () => ro.disconnect();
  }, []);

  // Click-away dismiss: when the card loses focus (user clicked another window/app), close it — so it's
  // not "always in front no matter what". Guarded by `hadFocus` so the initial show (before force_foreground
  // grants focus) can't self-close. Dragging/clicking chips stays within the window and doesn't blur it.
  useEffect(() => {
    const win = getCurrentWebviewWindow();
    let hadFocus = false;
    let unlisten: (() => void) | undefined;
    void win
      .onFocusChanged(({ payload: focused }) => {
        if (focused) {
          hadFocus = true;
        } else if (hadFocus && Date.now() >= suppressCloseUntilRef.current) {
          void commands.cancelReview();
        }
      })
      .then((u) => {
        unlisten = u;
      });
    return () => unlisten?.();
  }, []);

  const aiHint = t(
    "settings.review.aiHint",
    "Configure AI in Settings → Cleanup",
  );

  const chipLabel = (action: ReviewAction): string => {
    switch (action) {
      case "proofread":
        return t("settings.review.chip.proofread", "Proofread");
      case "rewrite":
        return t("settings.review.chip.rewrite", "Rewrite");
      case "formal":
        return t("settings.review.chip.formal", "Formal");
      case "summarize":
        return t("settings.review.chip.summarize", "Summarize");
    }
  };

  const renderChip = (action: ReviewAction) => {
    const enabled = chipEnabled(action, aiAvailable);
    const active = activeAction === action;
    return (
      <button
        key={action}
        type="button"
        disabled={!enabled}
        title={enabled ? undefined : aiHint}
        onClick={() => enabled && setActiveAction(action)}
        className={[
          "rounded-full px-3 py-1 text-xs font-medium transition-colors",
          active
            ? "bg-accent text-white"
            : "bg-inset text-muted hover:text-text",
          enabled ? "cursor-pointer" : "cursor-not-allowed opacity-40",
        ].join(" ")}
      >
        {chipLabel(action)}
      </button>
    );
  };

  return (
    <div
      ref={cardRef}
      className="font-sans flex w-screen flex-col overflow-hidden rounded-xl border border-hairline-strong bg-panel text-text"
    >
      {/* Drag handle — grab here to move the card (the window steals focus, so this needs an explicit
          drag region). Interactive children below aren't drag regions, so they still click normally. */}
      <div
        data-tauri-drag-region
        onMouseDown={() => {
          // Grab-to-move: suppress click-away-dismiss for ~1.2s so the drag's transient blur doesn't close it.
          suppressCloseUntilRef.current = Date.now() + 1200;
        }}
        className="flex cursor-move items-center justify-between border-b border-hairline px-3 py-1.5 select-none"
      >
        <span
          data-tauri-drag-region
          className="text-[11px] font-medium tracking-wide text-muted"
        >
          {t("settings.review.title", "Review")}
        </span>
      </div>

      {/* Chip row */}
      <div className="flex flex-wrap items-center gap-1.5 border-b border-hairline px-3 pt-2.5 pb-2.5">
        {renderChip("proofread")}
        {AI_ACTIONS.map(renderChip)}
      </div>

      {/* Body — grows with content up to CARD_MAX_HEIGHT, then scrolls internally. */}
      <div className="max-h-[360px] overflow-y-auto px-3 py-3 text-sm">
        {activeAction === "proofread" ? (
          <ReviewPanel text={text} onResult={setReviewResult} />
        ) : (
          <div className="rounded-lg border border-hairline bg-panel px-3 py-6 text-center text-sm text-muted">
            {t("settings.review.comingSoon", "AI actions coming soon")}
          </div>
        )}
      </div>

      {/* Footer */}
      <div className="flex items-center justify-end gap-2 border-t border-hairline px-3 py-2.5">
        <button
          type="button"
          onClick={() => void handleClose()}
          className="rounded-md px-3 py-1.5 text-xs font-medium text-muted hover:text-text"
        >
          {t("settings.review.close", "Close")}
        </button>
        <button
          type="button"
          onClick={() => void handleCopy()}
          className="rounded-md border border-hairline-strong px-3 py-1.5 text-xs font-medium text-text hover:bg-inset"
        >
          {t("settings.review.copy", "Copy")}
        </button>
        <button
          type="button"
          onClick={() => void handleApply()}
          className="rounded-md bg-accent px-3 py-1.5 text-xs font-medium text-white hover:brightness-95"
        >
          {t("settings.review.apply", "Apply")}
        </button>
      </div>
    </div>
  );
};

export default ReviewOverlay;
