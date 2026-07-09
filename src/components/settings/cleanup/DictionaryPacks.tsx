import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { Button } from "../../ui/Button";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { commands, type DictionaryPackInfo } from "@/bindings";

// DotFlow: dictionary packs — toggleable term lists that extend Harper's vocabulary so valid domain terms
// (drug names, standard abbreviations, anatomy, specialty words) stop being flagged as misspellings in the
// cleanup / review flows. The bundled "medical" pack ships with the app; users can drop additional *.txt
// packs into the dictionaries folder and hit Reload. Toggling rebuilds the merged dictionary live.

export const DictionaryPacks: React.FC = () => {
  const { t } = useTranslation();
  const [packs, setPacks] = useState<DictionaryPackInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const [reloading, setReloading] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setPacks(await commands.getDictionaryPacks());
    } catch {
      /* leave the last known list on transient failure */
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const togglePack = async (id: string, enabled: boolean) => {
    setBusy((b) => ({ ...b, [id]: true }));
    // Optimistic update so the switch responds immediately.
    setPacks((prev) =>
      prev.map((p) => (p.id === id ? { ...p, enabled } : p)),
    );
    try {
      await commands.setDictionaryPackEnabled(id, enabled);
    } catch {
      /* revert on failure */
      setPacks((prev) =>
        prev.map((p) => (p.id === id ? { ...p, enabled: !enabled } : p)),
      );
    } finally {
      setBusy((b) => ({ ...b, [id]: false }));
    }
  };

  const reload = async () => {
    setReloading(true);
    try {
      setPacks(await commands.reloadDictionaryPacks());
    } catch {
      /* keep the current list */
    } finally {
      setReloading(false);
    }
  };

  const openFolder = async () => {
    try {
      await commands.openDictionariesFolder();
    } catch {
      /* nothing actionable in the UI */
    }
  };

  return (
    <SettingsGroup
      title={t("settings.dictionaries.group", "Dictionaries")}
    >
      <div className="p-4 space-y-3">
        <p className="text-[13px] text-muted">
          {t(
            "settings.dictionaries.description",
            "Enable term packs so valid domain vocabulary (drug names, abbreviations, anatomy) is not flagged as a misspelling. Terms are never silently auto-corrected into — the safe, acceptance-only design.",
          )}
        </p>

        {loading ? (
          <div className="text-xs text-faint">
            {t("settings.dictionaries.loading", "Loading dictionary packs…")}
          </div>
        ) : packs.length === 0 ? (
          <div className="text-xs text-faint">
            {t("settings.dictionaries.empty", "No dictionary packs found.")}
          </div>
        ) : (
          <div className="space-y-2">
            {packs.map((pack) => (
              <ToggleSwitch
                key={pack.id}
                checked={pack.enabled}
                onChange={(v) => togglePack(pack.id, v)}
                isUpdating={busy[pack.id] ?? false}
                label={pack.label}
                description={t(
                  "settings.dictionaries.termCount",
                  "{{count}} terms",
                  { count: pack.term_count },
                )}
                grouped={true}
              />
            ))}
          </div>
        )}

        <div className="flex items-center gap-2">
          <Button variant="secondary" size="sm" onClick={openFolder}>
            {t("settings.dictionaries.openFolder", "Open dictionaries folder")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={reload}
            disabled={reloading}
          >
            {reloading
              ? t("settings.dictionaries.reloading", "Reloading…")
              : t("settings.dictionaries.reload", "Reload")}
          </Button>
          <span className="text-xs text-faint">
            {t(
              "settings.dictionaries.hint",
              "Drop your own .txt term lists (one term per line) in the folder, then Reload.",
            )}
          </span>
        </div>
      </div>
    </SettingsGroup>
  );
};
