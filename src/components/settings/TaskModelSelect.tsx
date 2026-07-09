import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingContainer } from "../ui/SettingContainer";
import { commands, type LocalModelInfo } from "@/bindings";

interface TaskModelSelectProps {
  /** Task role this override applies to (e.g. "transform"). */
  role: string;
  title: string;
  description: string;
  grouped?: boolean;
}

// DotFlow: pick a local model for a specific task (e.g. a small fast Gemma for quick transforms) while the
// chat/default model stays whatever you selected in the chat dropdown. Empty selection = "Same as chat model"
// (clears the override, so the task uses the default). Generalizes to future roles (SOAP, translate, …).
export const TaskModelSelect: React.FC<TaskModelSelectProps> = ({
  role,
  title,
  description,
  grouped = false,
}) => {
  const { t } = useTranslation();
  const [models, setModels] = useState<LocalModelInfo[]>([]);
  const [selected, setSelected] = useState<string>(""); // "" = same as default
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [list, current] = await Promise.all([
        commands.listLocalModels(),
        commands.getTaskModel(role),
      ]);
      setModels(list);
      setSelected(current);
    } catch {
      /* keep prior state */
    }
  }, [role]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const onChange = useCallback(
    async (path: string) => {
      setBusy(true);
      setSelected(path); // optimistic
      try {
        await commands.setTaskModel(role, path);
      } catch {
        await refresh();
      } finally {
        setBusy(false);
      }
    },
    [role, refresh],
  );

  return (
    <SettingContainer
      title={title}
      description={description}
      descriptionMode="tooltip"
      grouped={grouped}
    >
      <select
        className="max-w-[240px] rounded-md border border-hairline-strong bg-inset px-2 py-1 text-sm"
        value={selected}
        onChange={(e) => void onChange(e.target.value)}
        disabled={busy}
      >
        <option value="">
          {t("settings.taskModel.sameAsDefault", "Same as chat model")}
        </option>
        {models.map((m) => (
          <option key={m.path} value={m.path}>
            {m.name}
          </option>
        ))}
      </select>
    </SettingContainer>
  );
};
