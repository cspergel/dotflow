/* eslint-disable i18next/no-literal-string -- brand wordmark, never translated. */
import React from "react";

// DotFlow wordmark — "Dot" in the text color + "Flow" in the brand accent, led by a filled brand dot.
// (File/export name kept for import compatibility.) Sizes off `width` so the sidebar (120) and onboarding
// (200) both render crisply. Uses --color-logo-primary so it follows the theme (light + dark).
const HandyTextLogo = ({
  width = 120,
  height,
  className,
}: {
  width?: number;
  height?: number;
  className?: string;
}) => {
  const fs = width / 4.2; // font size derived from the intended pixel width
  const dot = fs * 0.44;
  return (
    <div
      className={className}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: fs * 0.2,
        height,
        fontSize: fs,
        fontWeight: 800,
        letterSpacing: "-0.03em",
        lineHeight: 1,
        whiteSpace: "nowrap",
      }}
    >
      <span
        style={{
          width: dot,
          height: dot,
          borderRadius: "9999px",
          background: "var(--color-logo-primary)",
          flexShrink: 0,
        }}
      />
      <span>
        Dot<span style={{ color: "var(--color-logo-primary)" }}>Flow</span>
      </span>
    </div>
  );
};

export default HandyTextLogo;
