import React from "react";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface TypedExpanderSoundProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

// DotFlow: play a short confirmation "ding" when the typed expander replaces a trigger. Independent of the
// dictation audio-feedback toggle. Only meaningful when the typed expander is enabled, so this control hides
// itself when the expander is off.
export const TypedExpanderSound: React.FC<TypedExpanderSoundProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const expanderOn = getSetting("experimental_typed_expander") ?? false;
    if (!expanderOn) return null;

    const enabled = getSetting("typed_expander_sound") ?? true;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(enabled) => updateSetting("typed_expander_sound", enabled)}
        isUpdating={isUpdating("typed_expander_sound")}
        label="Play a ding on expansion (DotFlow)"
        description="Play a short confirmation sound each time the typed expander replaces a trigger. Uses your selected output device and feedback volume."
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
