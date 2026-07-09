import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { Button } from "../../ui/Button";
import { Input } from "../../ui/Input";
import { commands } from "@/bindings";

// DotFlow: in-app editor for the user's custom accepted words ("My Words" pack, backed by
// dictionaries/custom.txt). Words added here flow through the exact same acceptance-only safety filter as any
// pack — they stop proofreading from flagging them, but are never silently auto-corrected *into*. Adding the
// first word auto-enables the pack, so it takes effect immediately with no restart.
export const CustomDictionaryWords: React.FC = () => {
  const { t } = useTranslation();
  const [words, setWords] = useState<string[]>([]);
  const [newWord, setNewWord] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setWords(await commands.getCustomDictionaryWords());
    } catch {
      /* keep the last known list */
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const addWord = useCallback(async () => {
    const word = newWord.trim();
    if (!word || busy) return;
    setBusy(true);
    setError(null);
    try {
      const res = await commands.addCustomDictionaryWord(word);
      if (res.status === "ok") {
        setWords(res.data);
        setNewWord("");
      } else {
        setError(res.error);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [newWord, busy]);

  const removeWord = useCallback(
    async (word: string) => {
      setBusy(true);
      // Optimistic: drop it right away so the chip feels responsive.
      setWords((prev) => prev.filter((w) => w !== word));
      try {
        const res = await commands.removeCustomDictionaryWord(word);
        if (res.status === "ok") setWords(res.data);
        else await load();
      } catch {
        await load();
      } finally {
        setBusy(false);
      }
    },
    [load],
  );

  return (
    <SettingsGroup title={t("settings.customDict.group", "My words")}>
      <div className="space-y-3 p-4">
        <p className="text-[13px] text-muted">
          {t(
            "settings.customDict.description",
            "Add your own words — names, jargon, product terms — so proofreading stops flagging them as misspellings. They're accepted, never auto-corrected into. Saved to your custom.txt pack.",
          )}
        </p>

        <div className="flex items-center gap-2">
          <Input
            type="text"
            className="max-w-64"
            value={newWord}
            onChange={(e) => {
              setNewWord(e.target.value);
              if (error) setError(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void addWord();
              }
            }}
            placeholder={t("settings.customDict.placeholder", "Add a word…")}
            variant="compact"
            disabled={busy}
          />
          <Button
            variant="primary"
            size="md"
            onClick={() => void addWord()}
            disabled={!newWord.trim() || busy}
          >
            {t("settings.customDict.add", "Add")}
          </Button>
        </div>

        {error && <p className="text-xs text-red-500">{error}</p>}

        {words.length > 0 ? (
          <div className="flex flex-wrap gap-1.5">
            {words.map((word) => (
              <button
                key={word}
                type="button"
                onClick={() => void removeWord(word)}
                disabled={busy}
                className="inline-flex items-center gap-1 rounded-md border border-hairline-strong bg-inset px-2 py-1 text-xs text-text hover:border-red-400/50 hover:text-red-500 disabled:opacity-50"
                title={t("settings.customDict.remove", "Remove {{word}}", {
                  word,
                })}
              >
                <span>{word}</span>
                <X size={11} />
              </button>
            ))}
          </div>
        ) : (
          <p className="text-xs text-faint">
            {t("settings.customDict.empty", "No custom words yet.")}
          </p>
        )}
      </div>
    </SettingsGroup>
  );
};
