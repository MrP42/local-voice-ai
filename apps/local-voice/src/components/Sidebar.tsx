import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Cog,
  History,
  Cpu,
  Users,
  Volume2,
  House,
  MoreHorizontal,
} from "lucide-react";
import { LocalVoiceAiMark } from "./icons/LocalVoiceAiLogo";
import {
  AppSettings,
  HistorySettings,
  ModelsSettings,
  TtsSettings,
  MeetingsSettings,
} from "./settings";

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
  models: {
    labelKey: "sidebar.models",
    icon: Cpu,
    component: ModelsSettings,
  },
  tts: {
    labelKey: "sidebar.tts",
    icon: Volume2,
    component: TtsSettings,
  },
  settings: {
    labelKey: "sidebar.settings",
    icon: Cog,
    component: AppSettings,
  },
} as const satisfies Record<string, SectionConfig>;

export const isSidebarSection = (value: string): value is SidebarSection =>
  Object.prototype.hasOwnProperty.call(SECTIONS_CONFIG, value);

interface SidebarProps {
  activeSection: SidebarSection;
  onSectionChange: (section: SidebarSection) => void;
}

export const Sidebar: React.FC<SidebarProps> = ({
  activeSection,
  onSectionChange,
}) => {
  const { t } = useTranslation();

  const [moreOpen, setMoreOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!moreOpen) return;
    const close = (event: PointerEvent) => {
      if (!moreRef.current?.contains(event.target as Node)) setMoreOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setMoreOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("pointerdown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", close);
      document.removeEventListener("keydown", escape);
    };
  }, [moreOpen]);

  const item = (id: SidebarSection) => {
    const { icon: Icon, labelKey } = SECTIONS_CONFIG[id];
    return (
      <button
        key={id}
        type="button"
        className="workspace-nav__item"
        aria-current={activeSection === id ? "page" : undefined}
        onClick={() => {
          onSectionChange(id);
          setMoreOpen(false);
        }}
      >
        <Icon size={20} aria-hidden="true" />
        <span>{t(labelKey)}</span>
      </button>
    );
  };
  return (
    <nav className="workspace-nav" aria-label={t("sidebar.ariaLabel")}>
      <div className="workspace-nav__brand">
        <LocalVoiceAiMark size={28} />
      </div>
      <div className="workspace-nav__primary">
        {(["home", "history", "meetings", "tts"] as const).map(item)}
      </div>
      <div className="workspace-nav__secondary" ref={moreRef}>
        <button
          ref={triggerRef}
          type="button"
          className="workspace-nav__item workspace-nav__more"
          aria-label={t("workspace.more")}
          aria-expanded={moreOpen}
          aria-controls="workspace-more"
          data-active={
            activeSection === "models" || activeSection === "settings"
          }
          onClick={() => setMoreOpen(!moreOpen)}
        >
          <MoreHorizontal size={20} aria-hidden="true" />
          <span>{t("workspace.more")}</span>
        </button>
        <div
          id="workspace-more"
          className="workspace-nav__utilities"
          data-open={moreOpen}
        >
          {(["models", "settings"] as const).map(item)}
        </div>
      </div>
    </nav>
  );
};
