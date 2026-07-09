import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { commands } from "@/bindings";

// DotFlow: let a reasoning model (Qwythos / Qwen3.x) think before answering AI transforms. Default OFF —
// transforms append `/no_think` for a fast, direct edit. ON omits it, so the model may reason (slower, but
// sometimes better on complex text). Self-contained get/set (no settings-store handler needed).
export const TransformReasoningToggle: React.FC = () => {
  const { t } = useTranslation();
  const [on, setOn] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    commands
      .getTransformReasoning()
      .then(setOn)
      .catch(() => {
        /* keep default */
      });
  }, []);

  const toggle = useCallback(
    async (v: boolean) => {
      setBusy(true);
      setOn(v); // optimistic
      try {
        const res = await commands.setTransformReasoning(v);
        if (res.status !== "ok") setOn(!v);
      } catch {
        setOn(!v);
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  return (
    <ToggleSwitch
      checked={on}
      onChange={toggle}
      isUpdating={busy}
      label={t("settings.transformReasoning.label", "Let the transforms model reason")}
      description={t(
        "settings.transformReasoning.description",
        "When on, a reasoning model (e.g. Qwythos) thinks before rewriting/summarizing — slower, but sometimes better on complex text. Off is fast and direct. Only affects reasoning models.",
      )}
      grouped={true}
    />
  );
};
