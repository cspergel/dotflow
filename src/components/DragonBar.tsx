import React from "react";
import { Maximize2, Mic, Minimize2 } from "lucide-react";
import HandyTextLogo from "./icons/HandyTextLogo";
import { useSettings } from "../hooks/useSettings";

// DotFlow compact bar — the Dragon-style always-on-top strip. Minimal: brand · status + expand.
// Mic puck is AMBER on standby, GREEN while dictating. The whole surface drags (data-tauri-drag-region on
// the container + pointer-events-none on the non-interactive children so clicks/drags pass through to it);
// only the expand button stays interactive. The shortcut reveals on hover.
const prettyKey = (binding?: string): string | null => {
  if (!binding) return null;
  return binding
    .split("+")
    .map((k) =>
      k
        .replace(/Left$|Right$/, "")
        .replace(/^Control$/, "Ctrl")
        .replace(/^Meta$/, "Cmd")
        .replace(/^Super$/, "Win")
        .trim(),
    )
    .filter(Boolean)
    .join(" + ");
};

interface DragonBarProps {
  onExpand: () => void;
  onShrink: () => void;
  isDictating: boolean;
}

export const DragonBar: React.FC<DragonBarProps> = ({
  onExpand,
  onShrink,
  isDictating,
}) => {
  const { settings } = useSettings();
  const key = prettyKey(settings?.bindings?.transcribe?.current_binding);
  const pushToTalk = settings?.push_to_talk ?? false;

  return (
    <div
      data-tauri-drag-region
      className="group h-screen w-screen flex items-center gap-2 px-3 select-none cursor-default"
    >
      {/* mic puck — amber on standby, green while dictating */}
      <div
        className={`flex items-center justify-center h-7 w-7 rounded-lg shrink-0 transition-colors pointer-events-none ${
          isDictating
            ? "bg-logo-primary text-white"
            : "bg-amber-400/20 text-amber-500"
        }`}
      >
        <Mic size={14} />
      </div>

      <div className="pointer-events-none">
        <HandyTextLogo width={60} />
      </div>

      <span className="text-mid-gray/40 pointer-events-none">·</span>

      <span className="flex items-center gap-1.5 min-w-0 pointer-events-none">
        <span
          className={`h-1.5 w-1.5 rounded-full shrink-0 ${isDictating ? "bg-logo-primary" : "bg-amber-400"}`}
        />
        <span
          className={`text-[11px] truncate ${isDictating ? "text-logo-primary font-medium" : "text-text/60"}`}
        >
          {isDictating ? "Listening…" : "Ready to dictate"}
        </span>
      </span>

      <div className="ms-auto flex items-center gap-1.5 shrink-0">
        {key && (
          <span className="flex items-center gap-1 text-[10px] text-text/50 opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
            {pushToTalk ? "Hold" : "Press"}
            <kbd className="px-1 py-0.5 rounded border border-mid-gray/30 bg-mid-gray/10 font-mono text-[9px] text-text/70">
              {key}
            </kbd>
          </span>
        )}
        <button
          onClick={onShrink}
          title="Shrink to mini bar"
          className="flex items-center justify-center h-6 w-6 rounded-md text-text/50 hover:bg-mid-gray/10 hover:text-logo-primary transition-colors"
        >
          <Minimize2 size={13} />
        </button>
        <button
          onClick={onExpand}
          title="Expand"
          className="flex items-center justify-center h-6 w-6 rounded-md text-text/50 hover:bg-mid-gray/10 hover:text-logo-primary transition-colors"
        >
          <Maximize2 size={13} />
        </button>
      </div>
    </div>
  );
};
