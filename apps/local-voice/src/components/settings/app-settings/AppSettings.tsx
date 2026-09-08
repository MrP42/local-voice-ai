import React from "react";
import { useTranslation } from "react-i18next";
import { DictationTab } from "./DictationTab";
import { ReadAloudTab } from "./ReadAloudTab";
import { SoundTab } from "./SoundTab";
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
 * are named after the question you arrive with — dictation, sound,
 * post-processing, the app itself — not after how deep a setting sits in the
 * code. The old split into "General" and "Advanced" told nobody where to look:
 * the microphone was general, the paste method advanced, and both belong to
 * the same act of dictating.
 *
 * Adding a setting means putting it in the group it belongs to, here. It does
 * not mean a new tab, and never a new sidebar entry. The dictation test sits at
 * the foot of the Diktat tab for the same reason: it checks what that tab
 * configures.
 */
const TABS = [
  {
    id: "dictation",
    labelKey: "settings.app.tabs.dictation",
    Component: DictationTab,
    enabled: () => true,
  },
  {
    // Direkt hinter dem Diktat: die beiden Dinge, die die App tut.
    id: "readaloud",
    labelKey: "settings.app.tabs.readAloud",
    Component: ReadAloudTab,
    enabled: () => true,
  },
  {
    id: "sound",
    labelKey: "settings.app.tabs.sound",
    Component: SoundTab,
    enabled: () => true,
  },
  {
    id: "postprocessing",
    labelKey: "settings.app.tabs.postProcessing",
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

export const AppSettings: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const [tab, setTab] = usePersistentState<TabId>(
    "settings.tab",
    "dictation",
    isTabId,
  );

  const available = TABS.filter((entry) => entry.enabled(settings));
  // A stored tab can point at one that is hidden again (debug switched off).
  const active = available.find((entry) => entry.id === tab) ?? available[0];
  const ActiveComponent = active.Component;

  return (
    <div className="w-full space-y-4">
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
    </div>
  );
};
