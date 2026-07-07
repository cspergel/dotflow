import React from "react";
import { Maximize2, Mic, Settings } from "lucide-react";
import HandyTextLogo from "./icons/HandyTextLogo";

// DotFlow SUPER-COMPACT bar — the smallest always-on-top strip: just the mic puck (amber on standby, green
// while dictating), the DotFlow wordmark, a settings shortcut, and an expand button. For users who want the
// tiniest possible footprint. The whole surface drags (data-tauri-drag-region + pointer-events-none on the
// non-interactive children); only the two buttons stay interactive.
interface MiniBarProps {
  /** Grow back to the normal compact bar. */
  onExpand: () => void;
  /** Jump straight to the full app / settings. */
  onSettings: () => void;
  isDictating: boolean;
}

export const MiniBar: React.FC<MiniBarProps> = ({
  onExpand,
  onSettings,
  isDictating,
}) => {
  const btn =
    "flex items-center justify-center h-6 w-6 rounded-md text-faint hover:bg-text/8 hover:text-accent transition-colors";

  return (
    <div
      data-tauri-drag-region
      className="group h-screen w-screen flex items-center gap-2 px-2.5 select-none cursor-default"
    >
      {/* mic puck — amber on standby, green while dictating */}
      <div
        className={`flex items-center justify-center h-6 w-6 rounded-lg shrink-0 transition-colors pointer-events-none ${
          isDictating
            ? "bg-accent text-white"
            : "bg-amber-400/20 text-amber-500"
        }`}
      >
        <Mic size={13} />
      </div>

      <div className="pointer-events-none">
        <HandyTextLogo width={54} />
      </div>

      <div className="ms-auto flex items-center gap-0.5 shrink-0">
        <button onClick={onSettings} title="Settings" className={btn}>
          <Settings size={13} />
        </button>
        <button onClick={onExpand} title="Expand" className={btn}>
          <Maximize2 size={12} />
        </button>
      </div>
    </div>
  );
};
