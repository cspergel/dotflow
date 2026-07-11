import { useEffect, useState, useRef } from "react";
import { toast, Toaster } from "sonner";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { platform } from "@tauri-apps/plugin-os";
import {
  checkAccessibilityPermission,
  checkMicrophonePermission,
} from "tauri-plugin-macos-permissions-api";
import { ModelStateEvent, RecordingErrorEvent } from "./lib/types/events";
import "./App.css";
import AccessibilityPermissions from "./components/AccessibilityPermissions";
import Footer from "./components/footer";
import Onboarding, { AccessibilityOnboarding } from "./components/onboarding";
import { Sidebar, SidebarSection, SECTIONS_CONFIG } from "./components/Sidebar";
import ChatView from "./components/chat/ChatView";
import QuickChat from "./components/chat/QuickChat";
import { OPEN_KEY, QUICK_CONV_KEY } from "./components/chat/chatStore";
import { TitleBar } from "./components/TitleBar";
import { DragonBar } from "./components/DragonBar";
import { MiniBar } from "./components/MiniBar";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { WhatsNewGate } from "./components/whats-new";
import { useSettings } from "./hooks/useSettings";
import { useSettingsStore } from "./stores/settingsStore";
import { commands } from "@/bindings";
import { getLanguageDirection, initializeRTL } from "@/lib/utils/rtl";

type OnboardingStep = "accessibility" | "model" | "done";

const renderSettingsContent = (section: SidebarSection) => {
  const ActiveComponent =
    SECTIONS_CONFIG[section]?.component || SECTIONS_CONFIG.general.component;
  return <ActiveComponent />;
};

function App() {
  const { t, i18n } = useTranslation();
  const [onboardingStep, setOnboardingStep] = useState<OnboardingStep | null>(
    null,
  );
  // Track if this is a returning user who just needs to grant permissions
  // (vs a new user who needs full onboarding including model selection)
  const [isReturningUser, setIsReturningUser] = useState(false);
  const [currentSection, setCurrentSection] =
    useState<SidebarSection>("general");
  // Three tiers, each resizing the window: the full app, the Dragon-style compact bar, and a super-compact
  // "mini" strip (mic + wordmark + settings/expand only).
  const [viewMode, setViewMode] = useState<"full" | "bar" | "mini">("full");
  // Live dictation state (backend emits on record start/stop) — colors the compact bars.
  const [isDictating, setIsDictating] = useState(false);
  // Quick-chat slide-out from the compact bar (grows the window when open).
  const [quickChatOpen, setQuickChatOpen] = useState(false);
  // True once the quick chat has been used since the last time we were in the full view — so expanding
  // continues in the AI chat even if the slide-out was closed again before expanding. Reset on view change.
  const quickChatUsedRef = useRef(false);

  const applyViewMode = async (mode: "full" | "bar" | "mini") => {
    setViewMode(mode);
    setQuickChatOpen(false);
    quickChatUsedRef.current = false;
    try {
      const win = getCurrentWindow();
      if (mode === "mini") {
        // Lower the min BEFORE shrinking, or the window is clamped to the current size.
        await win.setMinSize(new LogicalSize(150, 36));
        await win.setResizable(false);
        await win.setSize(new LogicalSize(178, 40));
        await win.setAlwaysOnTop(true);
      } else if (mode === "bar") {
        await win.setMinSize(new LogicalSize(300, 40));
        await win.setResizable(false);
        await win.setSize(new LogicalSize(360, 44));
        await win.setAlwaysOnTop(true);
      } else {
        await win.setAlwaysOnTop(false);
        await win.setResizable(true);
        await win.setMinSize(new LogicalSize(640, 520));
        await win.setSize(new LogicalSize(860, 640));
      }
    } catch (e) {
      console.warn("Failed to resize window for view mode:", e);
    }
  };

  // Expand from the compact bar to the full app. If the quick-chat slide-out is mid-conversation, continue
  // it in the full AI Chat view (hand the conversation id off) instead of returning to the prior section.
  const expandFromBar = () => {
    if (quickChatOpen || quickChatUsedRef.current) {
      try {
        const qid = localStorage.getItem(QUICK_CONV_KEY);
        if (qid) localStorage.setItem(OPEN_KEY, qid);
      } catch {
        /* best-effort handoff */
      }
      setCurrentSection("chat");
    }
    void applyViewMode("full");
  };

  // Toggle the quick-chat panel in the compact bar, growing/shrinking the window to fit it.
  const toggleQuickChat = async () => {
    const next = !quickChatOpen;
    setQuickChatOpen(next);
    if (next) quickChatUsedRef.current = true;
    try {
      const win = getCurrentWindow();
      await win.setSize(new LogicalSize(360, next ? 440 : 44));
    } catch (e) {
      console.warn("Failed to resize window for quick chat:", e);
    }
  };
  const { settings, updateSetting } = useSettings();
  const direction = getLanguageDirection(i18n.language);
  const refreshAudioDevices = useSettingsStore(
    (state) => state.refreshAudioDevices,
  );
  const refreshOutputDevices = useSettingsStore(
    (state) => state.refreshOutputDevices,
  );
  const hasCompletedPostOnboardingInit = useRef(false);

  useEffect(() => {
    checkOnboardingStatus();
  }, []);

  // Track live dictation state for the compact bar (backend emits true on record start, false on stop).
  useEffect(() => {
    const unlisten = listen<boolean>("dictation-state", (event) => {
      setIsDictating(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Initialize RTL direction when language changes
  useEffect(() => {
    initializeRTL(i18n.language);
  }, [i18n.language]);

  // Initialize Enigo, shortcuts, and refresh audio devices when main app loads
  useEffect(() => {
    if (onboardingStep === "done" && !hasCompletedPostOnboardingInit.current) {
      hasCompletedPostOnboardingInit.current = true;
      Promise.all([
        commands.initializeEnigo(),
        commands.initializeShortcuts(),
      ]).catch((e) => {
        console.warn("Failed to initialize:", e);
      });
      refreshAudioDevices();
      refreshOutputDevices();
    }
  }, [onboardingStep, refreshAudioDevices, refreshOutputDevices]);

  // Handle keyboard shortcuts for debug mode toggle
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      // Check for Ctrl+Shift+D (Windows/Linux) or Cmd+Shift+D (macOS)
      const isDebugShortcut =
        event.shiftKey &&
        event.key.toLowerCase() === "d" &&
        (event.ctrlKey || event.metaKey);

      if (isDebugShortcut) {
        event.preventDefault();
        const currentDebugMode = settings?.debug_mode ?? false;
        updateSetting("debug_mode", !currentDebugMode);
      }
    };

    // Add event listener when component mounts
    document.addEventListener("keydown", handleKeyDown);

    // Cleanup event listener when component unmounts
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [settings?.debug_mode, updateSetting]);

  // Listen for recording errors from the backend and show a toast
  useEffect(() => {
    const unlisten = listen<RecordingErrorEvent>("recording-error", (event) => {
      const { error_type, detail } = event.payload;

      if (error_type === "microphone_permission_denied") {
        const currentPlatform = platform();
        const platformKey = `errors.micPermissionDenied.${currentPlatform}`;
        const description = t(platformKey, {
          defaultValue: t("errors.micPermissionDenied.generic"),
        });
        toast.error(t("errors.micPermissionDeniedTitle"), { description });
      } else if (error_type === "no_input_device") {
        toast.error(t("errors.noInputDeviceTitle"), {
          description: t("errors.noInputDevice"),
        });
      } else {
        toast.error(
          t("errors.recordingFailed", { error: detail ?? "Unknown error" }),
        );
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  // Listen for paste failures and show a toast.
  // The technical error detail is logged to handy.log on the Rust side
  // (see actions.rs `error!("Failed to paste transcription: ...")`),
  // so we show a localized, user-friendly message here instead of the raw error.
  useEffect(() => {
    const unlisten = listen("paste-error", () => {
      toast.error(t("errors.pasteFailedTitle"), {
        description: t("errors.pasteFailed"),
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  // Listen for transcription failures and show a toast.
  // The payload is the backend error message (also logged to handy.log).
  useEffect(() => {
    const unlisten = listen<string>("transcription-error", (event) => {
      toast.error(t("errors.transcriptionFailedTitle"), {
        description: event.payload,
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  // Listen for model loading failures and show a toast
  useEffect(() => {
    const unlisten = listen<ModelStateEvent>("model-state-changed", (event) => {
      if (event.payload.event_type === "loading_failed") {
        toast.error(
          t("errors.modelLoadFailed", {
            model:
              event.payload.model_name || t("errors.modelLoadFailedUnknown"),
          }),
          {
            description: event.payload.error,
          },
        );
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  const revealMainWindowForPermissions = async () => {
    try {
      await commands.showMainWindowCommand();
    } catch (e) {
      console.warn("Failed to show main window for permission onboarding:", e);
    }
  };

  const checkOnboardingStatus = async () => {
    try {
      const settingsResult = await commands.getAppSettings();
      const hasCompletedOnboarding =
        settingsResult.status === "ok" &&
        settingsResult.data.onboarding_completed === true;
      const currentPlatform = platform();

      if (hasCompletedOnboarding) {
        // Returning user - check if they need to grant permissions first
        setIsReturningUser(true);

        if (currentPlatform === "macos") {
          try {
            const [hasAccessibility, hasMicrophone] = await Promise.all([
              checkAccessibilityPermission(),
              checkMicrophonePermission(),
            ]);
            if (!hasAccessibility || !hasMicrophone) {
              await revealMainWindowForPermissions();
              setOnboardingStep("accessibility");
              return;
            }
          } catch (e) {
            console.warn("Failed to check macOS permissions:", e);
            // If we can't check, proceed to main app and let them fix it there
          }
        }

        if (currentPlatform === "windows") {
          try {
            const microphoneStatus =
              await commands.getWindowsMicrophonePermissionStatus();
            if (
              microphoneStatus.supported &&
              microphoneStatus.overall_access === "denied"
            ) {
              await revealMainWindowForPermissions();
              setOnboardingStep("accessibility");
              return;
            }
          } catch (e) {
            console.warn("Failed to check Windows microphone permissions:", e);
            // If we can't check, proceed to main app and let them fix it there
          }
        }

        setOnboardingStep("done");
      } else {
        // New user - start full onboarding
        setIsReturningUser(false);
        setOnboardingStep("accessibility");
      }
    } catch (error) {
      console.error("Failed to check onboarding status:", error);
      setOnboardingStep("accessibility");
    }
  };

  const handleAccessibilityComplete = () => {
    // Returning users already have models, skip to main app
    // New users need to select a model
    setOnboardingStep(isReturningUser ? "done" : "model");
  };

  const handleModelSelected = () => {
    // Transition to main app - user has started a download
    setOnboardingStep("done");
  };

  // Still checking onboarding status
  if (onboardingStep === null) {
    return null;
  }

  if (onboardingStep === "accessibility") {
    return <AccessibilityOnboarding onComplete={handleAccessibilityComplete} />;
  }

  if (onboardingStep === "model") {
    return <Onboarding onModelSelected={handleModelSelected} />;
  }

  // Super-compact strip: mic + wordmark + settings/expand only.
  if (viewMode === "mini") {
    return (
      <div dir={direction} className="h-screen bg-background">
        <Toaster theme="system" />
        <MiniBar
          onExpand={() => applyViewMode("bar")}
          onSettings={() => applyViewMode("full")}
          isDictating={isDictating}
        />
      </div>
    );
  }

  // Dragon-style compact bar: dictation status + shortcut. Expands to the full app, or shrinks to mini.
  if (viewMode === "bar") {
    return (
      <div dir={direction} className="h-screen bg-background flex flex-col">
        <Toaster theme="system" />
        <div className="h-11 shrink-0">
          <DragonBar
            onExpand={expandFromBar}
            onShrink={() => applyViewMode("mini")}
            onToggleChat={() => void toggleQuickChat()}
            chatOpen={quickChatOpen}
            isDictating={isDictating}
          />
        </div>
        {quickChatOpen && (
          <div className="flex-1 min-h-0 border-t border-hairline">
            <QuickChat />
          </div>
        )}
      </div>
    );
  }

  return (
    <div
      dir={direction}
      className="h-screen flex flex-col select-none cursor-default"
    >
      <Toaster
        theme="system"
        toastOptions={{
          unstyled: true,
          classNames: {
            toast:
              "bg-panel border border-hairline rounded-xl shadow-lg px-4 py-3 flex items-center gap-3 text-sm",
            title: "font-medium",
            description: "text-muted",
          },
        }}
      />
      <WhatsNewGate />
      {/* Frameless custom titlebar: drag, dictation status, window controls + Compact toggle */}
      <TitleBar
        onCompact={() => applyViewMode("bar")}
        isDictating={isDictating}
      />
      {/* Main content area that takes remaining space */}
      <div className="flex-1 flex overflow-hidden">
        <Sidebar
          activeSection={currentSection}
          onSectionChange={setCurrentSection}
        />
        {/* Content area. The chat section renders full-bleed (fills the window, manages its own scroll,
            composer pinned to the bottom); all other sections use the centered, padded, scrollable layout. */}
        <div className="flex-1 flex flex-col overflow-hidden">
          {/* Chat stays MOUNTED (just hidden) when you navigate away, so an in-flight summarize / OCR / chat
              isn't lost — its result lands when you come back. Other sections mount/unmount normally. */}
          <div
            className={
              currentSection === "chat"
                ? "flex-1 min-h-0 overflow-hidden"
                : "hidden"
            }
          >
            <ChatView active={currentSection === "chat"} />
          </div>
          {currentSection !== "chat" && (
            <div className="flex-1 overflow-y-auto">
              <div className="flex flex-col items-center px-6 pt-6 pb-10 gap-4">
                <AccessibilityPermissions />
                {renderSettingsContent(currentSection)}
              </div>
            </div>
          )}
        </div>
      </div>
      {/* Fixed footer at bottom */}
      <Footer />
    </div>
  );
}

export default App;
