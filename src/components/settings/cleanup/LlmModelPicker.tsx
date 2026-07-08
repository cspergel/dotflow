import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { Button } from "../../ui/Button";
import { commands, type LlmModelInfo } from "@/bindings";

// DotFlow: the optional local-AI model picker. A curated catalog of small GGUF instruct models the user
// can download on demand to power the review overlay's Rewrite / Formal / Summarize actions fully
// offline. Selecting a model points settings.local_llm_model_path at its file, which is exactly what
// ai_transform reads — so no model = AI chips stay disabled (unless a cloud provider is configured).

interface DownloadProgressEvent {
  model_id: string;
  downloaded: number;
  total: number;
  percentage: number;
}

const formatSize = (bytes: number): string => {
  const gb = bytes / 1024 ** 3;
  if (gb >= 1) return `${gb.toFixed(gb >= 10 ? 0 : 1)} GB`;
  return `${Math.round(bytes / 1024 ** 2)} MB`;
};

export const LlmModelPicker: React.FC = () => {
  const { t } = useTranslation();
  const [models, setModels] = useState<LlmModelInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<Record<string, boolean>>({});

  const refresh = useCallback(async () => {
    try {
      setModels(await commands.listLlmModels());
    } catch {
      /* leave the last known list on transient failure */
    } finally {
      // First fetch resolved — stop showing the loading row (avoids an empty-flash on mount).
      setLoading(false);
    }
  }, []);

  const clearProgress = (id: string) =>
    setProgress((p) => {
      const next = { ...p };
      delete next[id];
      return next;
    });

  const failRow = (id: string, message: string) => {
    setErrors((x) => ({ ...x, [id]: message }));
    setBusy((b) => ({ ...b, [id]: false }));
    clearProgress(id);
  };

  const asMessage = (e: unknown): string =>
    e instanceof Error ? e.message : String(e);

  useEffect(() => {
    refresh();
  }, [refresh]);

  useEffect(() => {
    const unlisteners = [
      listen<DownloadProgressEvent>("llm-download-progress", (e) => {
        setProgress((p) => ({
          ...p,
          [e.payload.model_id]: e.payload.percentage,
        }));
      }),
      listen<string>("llm-download-complete", (e) => {
        clearProgress(e.payload);
        setBusy((b) => ({ ...b, [e.payload]: false }));
        refresh();
      }),
      listen<{ model_id: string; error: string }>(
        "llm-download-failed",
        (e) => {
          clearProgress(e.payload.model_id);
          setBusy((b) => ({ ...b, [e.payload.model_id]: false }));
          setErrors((x) => ({ ...x, [e.payload.model_id]: e.payload.error }));
        },
      ),
      // A cancelled download resets its row quietly — no error banner.
      listen<string>("llm-download-cancelled", (e) => {
        clearProgress(e.payload);
        setBusy((b) => ({ ...b, [e.payload]: false }));
        setErrors((x) => ({ ...x, [e.payload]: "" }));
      }),
      listen("llm-models-updated", () => refresh()),
    ];
    return () => {
      unlisteners.forEach((u) => u.then((fn) => fn()));
    };
  }, [refresh]);

  const download = async (id: string) => {
    setErrors((x) => ({ ...x, [id]: "" }));
    setBusy((b) => ({ ...b, [id]: true }));
    setProgress((p) => ({ ...p, [id]: 0 }));
    try {
      const res = await commands.downloadLlmModel(id);
      if (res.status === "error") {
        failRow(id, res.error);
      }
      // Success path finalizes via the llm-download-complete event.
    } catch (e) {
      // A rejected invoke (not just a Result-error) must also clear the busy/progress state.
      failRow(id, asMessage(e));
    }
  };

  // Cancel an in-flight download. The row is reset quietly by the llm-download-cancelled event;
  // we only surface a message if the cancel request itself fails.
  const cancel = async (id: string) => {
    try {
      const res = await commands.cancelLlmDownload(id);
      if (res.status === "error") {
        setErrors((x) => ({ ...x, [id]: res.error }));
      }
    } catch (e) {
      setErrors((x) => ({ ...x, [id]: asMessage(e) }));
    }
  };

  const select = async (id: string) => {
    try {
      const res = await commands.selectLlmModel(id);
      if (res.status === "error") {
        setErrors((x) => ({ ...x, [id]: res.error }));
      } else {
        setErrors((x) => ({ ...x, [id]: "" }));
        refresh();
      }
    } catch (e) {
      setErrors((x) => ({ ...x, [id]: asMessage(e) }));
    }
  };

  const remove = async (id: string) => {
    try {
      const res = await commands.deleteLlmModel(id);
      if (res.status === "error") {
        setErrors((x) => ({ ...x, [id]: res.error }));
      } else {
        setErrors((x) => ({ ...x, [id]: "" }));
        refresh();
      }
    } catch (e) {
      setErrors((x) => ({ ...x, [id]: asMessage(e) }));
    }
  };

  return (
    <SettingsGroup title={t("settings.llm.group", "Offline AI model")}>
      <div className="p-4 space-y-3">
        <p className="text-[13px] text-muted">
          {t(
            "settings.llm.description",
            "Download a small local model to power Rewrite / Formal / Summarize fully offline. No model selected means those AI actions stay disabled unless you configure a cloud provider under Post-Processing.",
          )}
        </p>

        {loading ? (
          <p className="text-[13px] text-muted">
            {t("settings.llm.loading", "Loading models…")}
          </p>
        ) : models.length === 0 ? (
          <p className="text-[13px] text-muted">
            {t("settings.llm.empty", "No models available.")}
          </p>
        ) : (
          models.map((m) => {
            const isDownloading = m.id in progress;
            const pct = Math.max(
              0,
              Math.min(100, Math.round(progress[m.id] ?? 0)),
            );
            return (
              <div
                key={m.id}
                className={`rounded-lg border p-3 space-y-2 ${
                  m.recommended
                    ? "border-accent/40 bg-accent-tint/40"
                    : "border-hairline bg-inset"
                }`}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="text-sm font-medium">{m.name}</span>
                      <span className="text-xs text-faint">{m.params}</span>
                      <span className="text-xs text-faint">
                        {formatSize(m.size_bytes)}
                      </span>
                      {m.recommended && (
                        <span className="text-xs font-medium px-2 py-0.5 rounded-md bg-accent-tint text-accent">
                          {t("settings.llm.recommended", "Recommended")}
                        </span>
                      )}
                      <span
                        className={`text-xs font-medium px-2 py-0.5 rounded-md ${
                          m.commercial_ok
                            ? "bg-inset text-muted"
                            : "bg-amber-500/15 text-amber-500"
                        }`}
                        title={
                          m.commercial_ok
                            ? t(
                                "settings.llm.commercialOk",
                                "Commercial use OK",
                              )
                            : t(
                                "settings.llm.nonCommercial",
                                "Non-commercial license — personal use only",
                              )
                        }
                      >
                        {m.license}
                        {!m.commercial_ok
                          ? ` · ${t("settings.llm.nonCommercialShort", "non-commercial")}`
                          : ""}
                      </span>
                    </div>
                    <p className="text-xs text-muted mt-1">{m.note}</p>
                  </div>

                  <div className="flex items-center gap-2 shrink-0">
                    {m.downloaded ? (
                      <>
                        <label className="flex items-center gap-1.5 text-xs cursor-pointer select-none">
                          <input
                            type="radio"
                            name="active-llm"
                            checked={m.active}
                            onChange={() => select(m.id)}
                          />
                          {m.active
                            ? t("settings.llm.active", "Active")
                            : t("settings.llm.use", "Use")}
                        </label>
                        <Button
                          variant="danger-ghost"
                          size="sm"
                          onClick={() => remove(m.id)}
                        >
                          {t("settings.llm.delete", "Delete")}
                        </Button>
                      </>
                    ) : isDownloading ? (
                      <>
                        <span className="text-xs text-muted whitespace-nowrap">
                          {t(
                            "settings.llm.downloading",
                            "Downloading… {{pct}}%",
                            { pct },
                          )}
                        </span>
                        <Button
                          variant="danger-ghost"
                          size="sm"
                          onClick={() => cancel(m.id)}
                        >
                          {t("settings.llm.cancel", "Cancel")}
                        </Button>
                      </>
                    ) : (
                      <Button
                        variant="primary"
                        size="sm"
                        disabled={busy[m.id]}
                        onClick={() => download(m.id)}
                      >
                        {t("settings.llm.download", "Download")}
                      </Button>
                    )}
                  </div>
                </div>

                {isDownloading && (
                  <div className="h-1.5 w-full rounded-full bg-hairline overflow-hidden">
                    <div
                      className="h-full bg-accent transition-[width] duration-200"
                      style={{ width: `${pct}%` }}
                    />
                  </div>
                )}

                {errors[m.id] && (
                  <p className="text-xs text-red-400">{errors[m.id]}</p>
                )}
              </div>
            );
          })
        )}
      </div>
    </SettingsGroup>
  );
};
