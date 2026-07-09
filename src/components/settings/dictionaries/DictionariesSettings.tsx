import React from "react";
import { useTranslation } from "react-i18next";
import { DictionaryPacks } from "../cleanup/DictionaryPacks";
import { CustomDictionaryWords } from "./CustomDictionaryWords";

// DotFlow: the Dictionaries tab — its own home (promoted out of Text Cleanup). Two things live here: the
// toggleable term packs (bundled Medical + any user .txt packs) and the in-app "My words" custom editor.
// Both feed the same offline Harper vocabulary used by cleanup, review, and the selection overlay.
export const DictionariesSettings: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div className="mx-auto w-full max-w-3xl space-y-6">
      <div>
        <h1 className="mb-1.5 text-xl font-semibold tracking-[-0.02em]">
          {t("settings.dictionaries.title", "Dictionaries")}
        </h1>
        <p className="text-[13px] text-muted">
          {t(
            "settings.dictionaries.pageDescription",
            "Extend the offline proofreading vocabulary so valid domain terms — drug names, jargon, product names — aren't flagged as misspellings. Terms are only ever accepted, never silently auto-corrected into.",
          )}
        </p>
      </div>

      <DictionaryPacks />
      <CustomDictionaryWords />
    </div>
  );
};
