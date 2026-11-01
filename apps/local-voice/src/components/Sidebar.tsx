import React from "react";
import { useTranslation } from "react-i18next";
import {
  Nav,
  NavItem,
  OverlayDrawer,
  DrawerHeader,
  DrawerHeaderTitle,
  Button as FluentButton,
  Tooltip as FluentTooltip,
} from "@fluentui/react-components";
import {
  Cog,
  History,
  Cpu,
  Phone,
  Plug,
  Users,
  Volume2,
  House,
  Library,
  UserRound,
  X,
} from "lucide-react";
import {
  AppSettings,
  HistorySettings,
  ModelsSettings,
  TtsSettings,
  MeetingsSettings,
} from "./settings";
import { APP_NAME } from "./shell/frame";
import { IntegrationsPage } from "./integrations/IntegrationsPage";
import { VoiceCallPage } from "./voice-call/VoiceCallPage";
import { KontoProfilSeite } from "./konto/KontoProfilSeite";
import { LibraryPage } from "./library/LibraryPage";

export type SidebarSection = keyof typeof SECTIONS_CONFIG;

interface IconProps {
  width?: number | string;
  height?: number | string;
  size?: number | string;
  className?: string;
  [key: string]: any;
}

interface SectionConfig {
  labelKey: string;
  icon: React.ComponentType<IconProps>;
  component: React.ComponentType;
}

// The sidebar lists what you DO with the app; everything that merely
// configures it lives under one "Einstellungen" entry with tabs (AppSettings).
// General, Advanced, the dictation test and the about page were four separate
// rows before — four rows of navigation for one activity.
export const SECTIONS_CONFIG = {
  home: { labelKey: "workspace.home", icon: House, component: () => null },
  history: {
    labelKey: "sidebar.history",
    icon: History,
    component: HistorySettings,
  },
  meetings: {
    labelKey: "workspace.recordings",
    icon: Users,
    component: MeetingsSettings,
  },
  voice: {
    labelKey: "sidebar.voiceCall",
    icon: Phone,
    component: VoiceCallPage,
  },
  models: {
    labelKey: "sidebar.models",
    icon: Cpu,
    component: ModelsSettings,
  },
  // A4 (#66): bewusste Ausnahme von "Einstellungen gehoeren in Einstellungen"
  // (Patrick, 30.09.): Verbindungen mit Richtung und Rechten brauchen eine Seite.
  integrations: {
    labelKey: "sidebar.integrations",
    icon: Plug,
    component: IntegrationsPage,
  },
  tts: {
    labelKey: "sidebar.tts",
    icon: Volume2,
    component: TtsSettings,
  },
  // lva#152: Bücher und Hörbücher, ein Eintrag je Werk.
  library: {
    labelKey: "sidebar.library",
    icon: Library,
    component: LibraryPage,
  },
  settings: {
    labelKey: "sidebar.settings",
    icon: Cog,
    component: AppSettings,
  },
  // ADR-0024: „Mein Profil“ öffnet über das Profilmenü oben rechts; die Seitenleiste
  // listet sie bewusst nicht (PRIMARY/UTILITIES nennen ihre Einträge einzeln).
  profile: {
    labelKey: "konto.meinProfil",
    icon: UserRound,
    component: KontoProfilSeite,
  },
} as const satisfies Record<string, SectionConfig>;

export const isSidebarSection = (value: string): value is SidebarSection =>
  Object.prototype.hasOwnProperty.call(SECTIONS_CONFIG, value);

interface SidebarProps {
  activeSection: SidebarSection;
  onSectionChange: (section: SidebarSection) => void;
  /** Eingeklappt: nur Symbole, Name als Tooltip und aria-label. */
  collapsed: boolean;
  /** Schmales Fenster: Navigation als Overlay-Drawer (Leitlinie §3). */
  narrow: boolean;
  drawerOpen: boolean;
  onDrawerOpenChange: (open: boolean) => void;
  /** Fuß der Navigation: der Profilkreis mit dem Profilmenü (ADR-0027 E1). */
  fuss?: React.ReactNode;
}

const PRIMARY = ["home", "history", "meetings", "tts", "library", "voice"] as const;
// Modelle, Integrationen und Einstellungen stehen seit ADR-0027 im Profilmenü
// (Profilkreis unten links), nicht mehr als Einträge der Navigation.

/**
 * Navigationsbereich nach Fluent 2 (Leitlinie windows-fluent.md §2/§3):
 * Fluent Nav mit Auswahl-Pill, nur die Arbeitsbereiche; unten der Profilkreis
 * (ADR-0027). Ein Tab-Stopp, darin Pfeiltasten. Eingeklappt bleibt die Symbolleiste (Hamburger in der
 * Titelleiste); schmal öffnet der Hamburger einen Overlay-Drawer.
 */
export const Sidebar: React.FC<SidebarProps> = ({
  activeSection,
  onSectionChange,
  collapsed,
  narrow,
  drawerOpen,
  onDrawerOpenChange,
  fuss,
}) => {
  const { t } = useTranslation();
  const iconOnly = collapsed && !narrow;

  const item = (id: SidebarSection) => {
    const { icon: Icon, labelKey } = SECTIONS_CONFIG[id];
    const label = t(labelKey);
    return (
      <NavItem
        key={id}
        value={id}
        className="workspace-nav__item"
        data-tour={`nav-${id}`}
        aria-current={activeSection === id ? "page" : undefined}
        // Eingeklappt bleibt nur das Symbol; der Name steht dann im Tooltip
        // (title) und für Screenreader im aria-label.
        title={iconOnly ? label : undefined}
        aria-label={iconOnly ? label : undefined}
      >
        <Icon size={20} aria-hidden="true" />
        <span className="workspace-nav__label">{label}</span>
      </NavItem>
    );
  };

  const nav = (
    <nav
      className="workspace-nav"
      aria-label={t("sidebar.ariaLabel")}
      data-collapsed={iconOnly}
      data-narrow={narrow || undefined}
    >
      <Nav
        selectedValue={activeSection}
        onNavItemSelect={(_event, data) => {
          onSectionChange(data.value as SidebarSection);
          if (narrow) onDrawerOpenChange(false);
        }}
        className="workspace-nav__fluent"
      >
        <div
          className="workspace-nav__primary"
          id="workspace-nav-items"
          data-tour="nav-primary"
        >
          {PRIMARY.map(item)}
        </div>
      </Nav>
      {fuss && <div className="workspace-nav__fuss">{fuss}</div>}
    </nav>
  );

  if (!narrow) return nav;

  return (
    <OverlayDrawer
      position="start"
      open={drawerOpen}
      onOpenChange={(_event, data) => onDrawerOpenChange(data.open)}
      // Ohne Ein-/Ausblend-Bewegung wie der Dialog: nach der Wahl eines Ziels
      // liegt kein ausblendender Hintergrund mehr über der neuen Seite.
      surfaceMotion={null}
      backdropMotion={null}
      aria-label={t("sidebar.ariaLabel")}
      className="workspace-nav__drawer"
    >
      <DrawerHeader>
        <DrawerHeaderTitle
          action={
            <FluentTooltip
              content={t("workspace.navClose")}
              relationship="label"
            >
              <FluentButton
                appearance="subtle"
                icon={<X size={20} aria-hidden="true" />}
                onClick={() => onDrawerOpenChange(false)}
              />
            </FluentTooltip>
          }
        >
          {APP_NAME}
        </DrawerHeaderTitle>
      </DrawerHeader>
      {nav}
    </OverlayDrawer>
  );
};
