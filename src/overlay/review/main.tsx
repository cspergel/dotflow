import React from "react";
import ReactDOM from "react-dom/client";
import ReviewOverlay from "./ReviewOverlay";
import "@/i18n";
// [F7] ReviewPanel is built from Tailwind v4 utilities, which are only emitted where
// `@import "tailwindcss"` exists. Import a stylesheet that pulls in Tailwind + the theme tokens so the
// review card renders styled (the recording overlay dodged this with hand-written CSS; the review UI
// does not).
import "./ReviewOverlay.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ReviewOverlay />
  </React.StrictMode>,
);
