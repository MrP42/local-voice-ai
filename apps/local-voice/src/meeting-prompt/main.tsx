import React from "react";
import ReactDOM from "react-dom/client";
import MeetingPrompt from "./MeetingPrompt";
import {
  applyTheme,
  getStoredTheme,
  syncThemeFromSettings,
} from "@/lib/utils/theme";
import "@/i18n";
import "@/App.css";

// Wie das Hauptfenster: zuletzt bekannte Palette sofort, dann mit der
// gespeicherten Einstellung abgleichen (kein Aufblitzen der falschen Farben).
applyTheme(getStoredTheme());
syncThemeFromSettings();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <MeetingPrompt />
  </React.StrictMode>,
);
