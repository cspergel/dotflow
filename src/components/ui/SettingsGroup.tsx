import React from "react";

interface SettingsGroupProps {
  title?: string;
  description?: string;
  children: React.ReactNode;
}

export const SettingsGroup: React.FC<SettingsGroupProps> = ({
  title,
  description,
  children,
}) => {
  return (
    <div className="space-y-2.5">
      {title && (
        <div className="px-1">
          <h2 className="text-[11px] font-semibold text-faint uppercase tracking-[0.08em]">
            {title}
          </h2>
          {description && (
            <p className="text-[13px] text-muted mt-1">{description}</p>
          )}
        </div>
      )}
      <div className="bg-panel border border-hairline rounded-xl overflow-visible">
        <div className="divide-y divide-hairline">{children}</div>
      </div>
    </div>
  );
};
