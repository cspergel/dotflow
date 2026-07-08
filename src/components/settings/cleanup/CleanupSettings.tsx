import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { Button } from "../../ui/Button";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { ShortcutInput } from "../ShortcutInput";
import { ReviewPanel } from "./ReviewPanel";
import { LlmModelPicker } from "./LlmModelPicker";
import { useSettings } from "../../../hooks/useSettings";
import { commands } from "@/bindings";

// DotFlow: the Text Cleanup section — the home for the "clean up selected text" hotkey and (soon) its
// dictionaries, post-dictation auto-clean, and live trailing corrector. The "Try it" box runs the exact
// cleanup pipeline the hotkey uses, so it can be used/verified without the global hotkey.
export const CleanupSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const reviewEnabled = getSetting("selection_review_enabled") ?? true;
  const [aiConfigured, setAiConfigured] = useState(false);
  const [input, setInput] = useState("this is an  test ,it has recieve  erors");
  const [output, setOutput] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Interactive review: snapshot the input when "Review" is clicked so editing the box later doesn't
  // re-analyze on every keystroke.
  const [reviewText, setReviewText] = useState<string | null>(null);
  const [reviewResult, setReviewResult] = useState("");
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    commands
      .postProcessIsConfigured()
      .then(setAiConfigured)
      .catch(() => setAiConfigured(false));
  }, []);

  const runCleanup = async () => {
    setReviewText(null);
    setBusy(true);
    try {
      const res = await commands.previewCleanup(input);
      if (res.status === "ok") {
        setOutput(res.data);
      } else {
        setOutput(`Error: ${res.error}`);
      }
    } catch (e) {
      setOutput(`Error: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const startReview = () => {
    setOutput(null);
    setReviewResult(input);
    setReviewText(input);
  };

  const copyResult = async () => {
    try {
      await navigator.clipboard.writeText(reviewResult);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard unavailable — the user can select the text */
    }
  };

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <div>
        <h1 className="text-xl font-semibold tracking-[-0.02em] mb-1.5">
          {t("settings.cleanup.title", "Text Cleanup")}
        </h1>
        <p className="text-[13px] text-muted">
          {t(
            "settings.cleanup.description",
            "Clean up selected text in any app with a global hotkey — spelling, grammar, and formatting. Works fully offline; upgrades automatically if you configure an AI provider.",
          )}
        </p>
      </div>

      <SettingsGroup title={t("settings.cleanup.groups.hotkey", "Hotkey")}>
        <ShortcutInput shortcutId="cleanup_selection" grouped={true} />
        <SettingContainer
          title={t("settings.cleanup.engine.title", "Cleanup engine")}
          description={t(
            "settings.cleanup.engine.description",
            "Offline grammar & spelling (Harper) by default. Configure an AI provider under Post-Processing for a fuller, context-aware cleanup — the hotkey uses it automatically.",
          )}
          descriptionMode="tooltip"
          grouped={true}
        >
          <span
            className={`text-xs font-medium px-2.5 py-1 rounded-md whitespace-nowrap ${
              aiConfigured
                ? "bg-accent-tint text-accent"
                : "bg-inset text-muted"
            }`}
          >
            {aiConfigured
              ? t("settings.cleanup.engine.ai", "AI post-processing")
              : t("settings.cleanup.engine.offline", "Offline (Harper)")}
          </span>
        </SettingContainer>
      </SettingsGroup>

      <SettingsGroup title={t("settings.review.group", "Selection review")}>
        <ToggleSwitch
          checked={reviewEnabled}
          onChange={(v) => updateSetting("selection_review_enabled", v)}
          isUpdating={isUpdating("selection_review_enabled")}
          label={t("settings.review.enable", "Selection review overlay")}
          description={t(
            "settings.review.enableDesc",
            "A hotkey opens a floating review card near the cursor to proofread the selected text before pasting it back. Rewrite/Formal/Summarize require an AI provider configured under Post-Processing.",
          )}
          grouped={true}
        />
        {reviewEnabled && (
          <ShortcutInput shortcutId="review_selection" grouped={true} />
        )}
      </SettingsGroup>

      <LlmModelPicker />

      <SettingsGroup title={t("settings.cleanup.groups.tryIt", "Try it")}>
        <div className="p-4 space-y-3">
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            rows={3}
            spellCheck={false}
            className="w-full px-3 py-2 text-sm bg-inset border border-hairline-strong rounded-lg focus:outline-none focus:ring-2 focus:ring-accent/35 resize-y"
          />
          <div className="flex items-center gap-2">
            <Button
              variant="primary"
              size="md"
              onClick={startReview}
              disabled={input.trim().length === 0}
            >
              {t("settings.cleanup.review", "Review")}
            </Button>
            <Button
              variant="secondary"
              size="md"
              onClick={runCleanup}
              disabled={busy || input.trim().length === 0}
            >
              {busy
                ? t("settings.cleanup.cleaning", "Fixing…")
                : t("settings.cleanup.clean", "Auto-fix")}
            </Button>
            <span className="text-xs text-faint">
              {t(
                "settings.cleanup.tryHint",
                "Review = accept each fix; Auto-fix = clean it all.",
              )}
            </span>
          </div>

          {reviewText !== null && (
            <div className="space-y-3">
              <ReviewPanel text={reviewText} onResult={setReviewResult} />
              <div className="flex items-center gap-2">
                <Button variant="primary" size="sm" onClick={copyResult}>
                  {copied
                    ? t("settings.cleanup.copied", "Copied ✓")
                    : t("settings.cleanup.copyResult", "Copy result")}
                </Button>
                <span className="text-xs text-faint">
                  {t(
                    "settings.cleanup.reviewHint",
                    "Click an underlined word, or use the cards below.",
                  )}
                </span>
              </div>
            </div>
          )}

          {output !== null && (
            <div className="px-3 py-2 text-sm bg-panel border border-hairline rounded-lg whitespace-pre-wrap">
              {output || t("settings.cleanup.noOutput", "(no change)")}
            </div>
          )}
        </div>
      </SettingsGroup>
    </div>
  );
};
