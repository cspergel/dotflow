import React from "react";
import { Minus, X, Minimize2 } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import HandyTextLogo from "./icons/HandyTextLogo";
import { useSettings } from "../hooks/useSettings";

// DotFlow custom titlebar (the window is frameless). Draggable, with the wordmark + live dictation status on
// the left and window controls on the right. Close hides to the tray (matches DotFlow's background behavior).
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

interface TitleBarProps {
  onCompact: () => void;
  isDictating: boolean;
}

export const TitleBar: React.FC<TitleBarProps> = ({
  onCompact,
  isDictating,
}) => {
  const { settings } = useSettings();
  const key = prettyKey(settings?.bindings?.transcribe?.current_binding);
  const win = getCurrentWindow();

  const ctrlBtn =
    "flex items-center justify-center h-7 w-7 rounded-md text-faint hover:bg-text/8 hover:text-text transition-colors";

  return (
    <div
      data-tauri-drag-region
      className="flex items-center gap-3 px-3 h-12 border-b border-hairline shrink-0 select-none"
    >
      <HandyTextLogo width={92} />

      <div className="flex items-center gap-1.5 ms-1 pointer-events-none">
        <span className="relative flex h-2 w-2">
          <span
            className={`relative inline-flex h-2 w-2 rounded-full ${
              isDictating ? "bg-logo-primary" : "bg-amber-400"
            }`}
          />
        </span>
        <span
          className={`text-xs ${isDictating ? "text-accent font-medium" : "text-muted"}`}
        >
          {isDictating ? "Listening…" : "Ready to dictate"}
        </span>
        {key && !isDictating && (
          <kbd className="ms-1 px-1.5 py-0.5 rounded border border-hairline-strong bg-inset font-mono text-[10px] text-muted">
            {key}
          </kbd>
        )}
      </div>

      <div className="ms-auto flex items-center gap-0.5">
        <button onClick={onCompact} title="Compact mode" className={ctrlBtn}>
          <Minimize2 size={14} />
        </button>
        <button
          onClick={() => win.minimize()}
          title="Minimize"
          className={ctrlBtn}
        >
          <Minus size={16} />
        </button>
        <button
          onClick={() => win.hide()}
          title="Close to tray"
          className="flex items-center justify-center h-7 w-7 rounded-md text-text/60 hover:bg-red-500 hover:text-white transition-colors"
        >
          <X size={15} />
        </button>
      </div>
    </div>
  );
};
