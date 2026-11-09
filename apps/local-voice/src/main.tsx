import React from "react";
import ReactDOM from "react-dom/client";
import { platform } from "@tauri-apps/plugin-os";
import App from "./App";
import { FluentRoot } from "./fluent/FluentRoot";
import { migrateLocalAppearance } from "./fluent/appearance";
import { richteMacFensterEin } from "./lib/platform/macFenster";

// Set platform before render so CSS can scope per-platform (e.g. scrollbar styles)
document.documentElement.dataset.platform = platform();
// Mac: Doppelklick auf die Titelleiste laut Systemeinstellung, Vollbild (lva#438).
richteMacFensterEin();

// Erscheinungsbild: zuletzt bekannten Stand sofort (localStorage, bei Bedarf
// aus dem alten `handy.theme` übernommen); den Abgleich mit AppSettings
// macht FluentRoot.
migrateLocalAppearance();

// Initialize i18n
import "./i18n";

// Initialize model store (loads models and sets up event listeners)
import { useModelStore } from "./stores/modelStore";
useModelStore.getState().initialize();

// Initialize Piper (Vorlesestimmen) download store — same pattern (Paket B-E3)
import { useTtsModelStore } from "./stores/ttsModelStore";
useTtsModelStore.getState().initialize();

// Fortschritt (Weiterlesen, Gehörtes, Lesezeichen) aus dem App-Kern abgleichen,
// damit er einen geleerten WebView-Speicher übersteht (lva#152).
import {
  adoptSyncedProgress,
  hydrateProgress,
  removeSyncedKeys,
} from "./lib/library/progress";
import { listen } from "@tauri-apps/api/event";
void hydrateProgress();
// Geräte-Sync (lva#210): Fortschritt und Lesezeichen anderer Geräte übernehmen.
void listen("sync-changed", () => void adoptSyncedProgress()).catch(() => {});
void listen<string[]>("sync-progress-removed", (e) =>
  removeSyncedKeys(Array.isArray(e.payload) ? e.payload : []),
).catch(() => {});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <FluentRoot>
      <App />
    </FluentRoot>
  </React.StrictMode>,
);
