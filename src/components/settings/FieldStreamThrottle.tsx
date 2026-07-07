import React from "react";
import { Slider } from "../ui/Slider";
import { useSettings } from "../../hooks/useSettings";

interface FieldStreamThrottleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

// DotFlow: fine-tune the spacing between live field-injection writes. Higher = safer against the
// dropped/repeated-key race (bigger, less frequent bursts); lower = more immediate but riskier on machines
// where the OS input queue can't keep up. Only meaningful when Live field streaming is on.
export const FieldStreamThrottle: React.FC<FieldStreamThrottleProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { getSetting, updateSetting } = useSettings();

    const streamingOn = getSetting("experimental_field_streaming") ?? false;
    const throttleMs = getSetting("field_stream_throttle_ms") ?? 100;

    return (
      <Slider
        value={throttleMs}
        onChange={(value: number) =>
          updateSetting("field_stream_throttle_ms", Math.round(value))
        }
        min={0}
        max={400}
        step={10}
        label="Field streaming throttle"
        description="Minimum spacing between live keystroke writes. Raise it if you see dropped or repeated characters (e.g. 'kkkke'); lower it for a more immediate feel."
        descriptionMode={descriptionMode}
        grouped={grouped}
        formatValue={(value) => `${Math.round(value)} ms`}
        disabled={!streamingOn}
      />
    );
  });
