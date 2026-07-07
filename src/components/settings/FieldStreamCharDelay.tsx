import React from "react";
import { Slider } from "../ui/Slider";
import { useSettings } from "../../hooks/useSettings";

interface FieldStreamCharDelayProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

// DotFlow: per-character delay for keystroke injection. The main lever if injected text shows dropped or
// repeated characters ("ggggg") — raise it until typing is clean, lower it for faster typing.
export const FieldStreamCharDelay: React.FC<FieldStreamCharDelayProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { getSetting, updateSetting } = useSettings();

    const streamingOn = getSetting("experimental_field_streaming") ?? false;
    const delayMs = getSetting("field_stream_char_delay_ms") ?? 8;

    return (
      <Slider
        value={delayMs}
        onChange={(value: number) =>
          updateSetting("field_stream_char_delay_ms", Math.round(value))
        }
        min={0}
        max={40}
        step={1}
        label="Typing delay per character"
        description="Delay between injected characters. Raise it if letters drop or repeat ('ggggg'); lower it for faster typing."
        descriptionMode={descriptionMode}
        grouped={grouped}
        formatValue={(value) => `${Math.round(value)} ms`}
        disabled={!streamingOn}
      />
    );
  });
