import React from "react";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface TypedExpanderProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

// DotFlow: the Beeftext/Espanso-style TYPED text expander. When on, a global keyboard monitor watches what
// you type in ANY app and replaces your dot-triggers (e.g. `.fu`) with the saved phrase — the same library
// that powers spoken triggers. It monitors typing, so it is strictly opt-in (default OFF). Windows-only for
// now. Emits by backspacing the trigger and pasting the expansion; it suppresses its own keystrokes so it
// can never re-trigger itself.
export const TypedExpander: React.FC<TypedExpanderProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const enabled = getSetting("experimental_typed_expander") ?? false;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(enabled) =>
          updateSetting("experimental_typed_expander", enabled)
        }
        isUpdating={isUpdating("experimental_typed_expander")}
        label="Typed text expander (DotFlow)"
        description="Monitors your typing in any app and expands your dot-triggers (e.g. type “.fu” → your saved phrase), using the same library as spoken triggers. Windows only for now. Off by default — it watches your keyboard, so enable it only if you want typed expansion."
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
