import React from "react";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface FieldStreamingProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

// DotFlow: live word-by-word injection of committed text into the focused field as you speak (the Dragon
// feel). Requires a STREAMING model (e.g. Parakeet Unified) that emits committed/tentative text. Injects
// completed words in batched enigo bursts; the tentative guess stays in the overlay.
export const FieldStreaming: React.FC<FieldStreamingProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const enabled = getSetting("experimental_field_streaming") ?? false;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(enabled) =>
          updateSetting("experimental_field_streaming", enabled)
        }
        isUpdating={isUpdating("experimental_field_streaming")}
        label="Live field streaming (DotFlow)"
        description="Type text into the focused field word-by-word as you speak, instead of pasting it all at the end. Requires a streaming model (e.g. Parakeet Unified)."
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
