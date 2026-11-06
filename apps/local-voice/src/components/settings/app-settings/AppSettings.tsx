import React, { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { PageShell } from "../../ui/PageShell";
import { TabList } from "../../ui/TabList";
import { DictationTab } from "./DictationTab";
import { OutputTab } from "./OutputTab";
import { AppTab } from "./AppTab";
import { AccountTab } from "./AccountTab";
import { PostProcessingSettings } from "../post-processing/PostProcessingSettings";
import { AboutSettings } from "../about/AboutSettings";
import { DebugSettings } from "../debug/DebugSettings";
import { useSettings } from "../../../hooks/useSettings";
import { usePersistentState } from "../../../hooks/usePersistentState";
import {
  OPEN_SETTINGS_TAB,
  scrollToPendingAnchor,
} from "../../../lib/openSettingsTab";

/**
 * "Einstellungen" — the single place where the app is configured.
 *
 * The sidebar lists what you DO (history, meetings, models, read aloud);
 * everything that merely configures the app is one entry with tabs. The tabs
 * are sorted by topic, not by feature module: Eingabe (what goes in: dictation,
 * microphone, text enhancement), Ausgabe (what comes out: read aloud, sounds),
 * KI-Modelle & Anbieter (which language model, from whom, at what cost), the
 * app itself and Konto & Geräte (profiles on this PC, device sync; ADR-0024).
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
    id: "account",
    labelKey: "settings.app.tabs.account",
    Component: AccountTab,
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

  // Sprung von ausserhalb (Fussleiste: "Anbieter-Einstellungen"): Reiter
  // setzen und zum vorgemerkten Anker scrollen. Beim ersten Oeffnen liest
  // usePersistentState den Reiter selbst; der Anker wartet auf diese Seite.
  useEffect(() => {
    const onOpen = (event: Event) => {
      const next = (event as CustomEvent<{ tab?: string }>).detail?.tab;
      if (next && isTabId(next)) setTab(next);
      scrollToPendingAnchor();
    };
    window.addEventListener(OPEN_SETTINGS_TAB, onOpen);
    scrollToPendingAnchor();
    return () => window.removeEventListener(OPEN_SETTINGS_TAB, onOpen);
  }, [setTab]);

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
      <TabList
        tabs={available.map((entry) => ({
          id: entry.id,
          label: t(entry.labelKey),
        }))}
        value={active.id}
        onChange={(id) => setTab(id)}
        ariaLabel={t("sidebar.settings")}
        idPrefix="settings-tab"
        controls="settings-panel"
        className="overflow-x-auto"
      />
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
