import React from "react";
import { useTranslation } from "react-i18next";
import { PageShell } from "../../ui/PageShell";
import { DictationTab } from "./DictationTab";
import { OutputTab } from "./OutputTab";
import { AppTab } from "./AppTab";
import { PostProcessingSettings } from "../post-processing/PostProcessingSettings";
import { AboutSettings } from "../about/AboutSettings";
import { DebugSettings } from "../debug/DebugSettings";
import { useSettings } from "../../../hooks/useSettings";
import { usePersistentState } from "../../../hooks/usePersistentState";

/**
 * "Einstellungen" — the single place where the app is configured.
 *
 * The sidebar lists what you DO (history, meetings, models, read aloud);
 * everything that merely configures the app is one entry with tabs. The tabs
 * are sorted by topic, not by feature module: Eingabe (what goes in: dictation,
 * microphone, text enhancement), Ausgabe (what comes out: read aloud, sounds),
 * KI-Modelle & Anbieter (which language model, from whom, at what cost) and
 * the app itself.
 *
 * Adding a setting means putting it in the group it belongs to, here. It does
 * not mean a new tab, and never a new sidebar entry. The dictation test sits at
 * the foot of the Diktat tab for the same reason: it checks what that tab
 * configures.
 */
const TABS = [
  {
    id: "input",
    labelKey: "settings.app.tabs.input",
    Component: DictationTab,
    enabled: () => true,
  },
  {
    id: "output",
    labelKey: "settings.app.tabs.output",
    Component: OutputTab,
    enabled: () => true,
  },
  {
    id: "models",
    labelKey: "settings.app.tabs.models",
    Component: PostProcessingSettings,
    enabled: () => true,
  },
  {
    id: "app",
    labelKey: "settings.app.tabs.app",
    Component: AppTab,
    enabled: () => true,
  },
  {
    id: "about",
    labelKey: "settings.app.tabs.about",
    Component: AboutSettings,
    enabled: () => true,
  },
  {
    id: "debug",
    labelKey: "sidebar.debug",
    Component: DebugSettings,
    enabled: (settings: any) => settings?.debug_mode ?? false,
  },
] as const;

type TabId = (typeof TABS)[number]["id"];

const isTabId = (value: string): value is TabId =>
  TABS.some((tab) => tab.id === value);

/** Reiter vor der Neugliederung (09.10.2026) -> der Reiter, der ihren Inhalt jetzt traegt. */
const LEGACY_TABS: Record<string, TabId> = {
  dictation: "input",
  readaloud: "output",
  sound: "output",
  postprocessing: "models",
};

/** Schreibt einen gespeicherten alten Reiter einmalig auf den neuen um. */
const migrateLegacyTab = () => {
  try {
    const key = "lva.ui.settings.tab";
    const stored = window.localStorage.getItem(key);
    if (stored && stored in LEGACY_TABS) {
      window.localStorage.setItem(key, LEGACY_TABS[stored]);
    }
  } catch {
    /* ohne Speicher gilt der Vorgabe-Reiter */
  }
};

export const AppSettings: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  // Vor dem ersten Lesen des gespeicherten Reiters; der Initialisierer von
  // useState laeuft nur einmal.
  React.useState(migrateLegacyTab);
  const [tab, setTab] = usePersistentState<TabId>(
    "settings.tab",
    "input",
    isTabId,
  );

  const available = TABS.filter((entry) => entry.enabled(settings));
  // A stored tab can point at one that is hidden again (debug switched off).
  const active = available.find((entry) => entry.id === tab) ?? available[0];
  const ActiveComponent = active.Component;

  return (
    <PageShell
      title={t("sidebar.settings")}
      description={t("workspace.settingsHint")}
      help="einstellungen"
    >
      {/* Scrolls rather than wraps: on a narrow window a wrapped strip pushes
          the content down by a whole row for no gain. */}
      <div
        role="tablist"
        aria-label={t("sidebar.settings")}
        className="flex gap-1 border-b border-mid-gray/20 overflow-x-auto"
      >
        {available.map((entry) => (
          <button
            key={entry.id}
            type="button"
            role="tab"
            id={`settings-tab-${entry.id}`}
            aria-controls="settings-panel"
            tabIndex={entry.id === active.id ? 0 : -1}
            onKeyDown={(event) => {
              const keys = ["ArrowRight", "ArrowLeft", "Home", "End"];
              if (!keys.includes(event.key)) return;
              event.preventDefault();
              const index = available.findIndex((item) => item.id === entry.id);
              const rtl =
                event.currentTarget.closest("[dir]")?.getAttribute("dir") ===
                "rtl";
              const step =
                (event.key === "ArrowRight" ? 1 : -1) * (rtl ? -1 : 1);
              const next =
                event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? available.length - 1
                    : (index + step + available.length) % available.length;
              setTab(available[next].id);
              document
                .getElementById(`settings-tab-${available[next].id}`)
                ?.focus();
            }}
            aria-selected={entry.id === active.id}
            onClick={() => setTab(entry.id)}
            /* `first:pl-0`: der erste Reiter beginnt buendig mit den Karten
               darunter. Mit Innenabstand sass seine Beschriftung elf Pixel
               weiter rechts als jeder Inhalt der Seite — genug, dass die
               Leiste verrutscht aussieht, zu wenig, um wie Absicht zu
               wirken. */
            className={`min-h-11 px-3 first:pl-0 py-2 text-sm font-medium border-b-2 cursor-pointer whitespace-nowrap transition-colors ${
              entry.id === active.id
                ? "border-logo-primary text-text"
                : "border-transparent text-text/60 hover:text-text"
            }`}
          >
            {t(entry.labelKey)}
          </button>
        ))}
      </div>
      <div
        role="tabpanel"
        id="settings-panel"
        aria-labelledby={`settings-tab-${active.id}`}
      >
        <ActiveComponent />
      </div>
    </PageShell>
  );
};
