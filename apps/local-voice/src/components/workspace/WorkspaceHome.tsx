import {
  ArrowRight,
  Mic,
  History,
  Users,
  Volume2,
  Cpu,
  Settings,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useSettings } from "@/hooks/useSettings";
import type { SidebarSection } from "../Sidebar";

export function WorkspaceHome({
  onNavigate,
}: {
  onNavigate: (section: SidebarSection) => void;
}) {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const shortcut = getSetting("bindings")?.transcribe?.current_binding;
  const openDictationSettings = () => {
    try {
      localStorage.setItem("lva.ui.settings.tab", "dictation");
    } catch {
      /* Navigation still works if browser storage is unavailable. */
    }
    onNavigate("settings");
  };
  const tasks = [
    { id: "meetings", icon: Users, key: "meetings" },
    { id: "history", icon: History, key: "history" },
    { id: "tts", icon: Volume2, key: "tts" },
  ] as const;
  return (
    <div className="workspace-home">
      <header>
        <h1 className="text-lg font-semibold">{t("workspace.title")}</h1>
        <p className="text-sm text-text/70 mt-1">{t("workspace.intro")}</p>
      </header>
      <section
        className="workspace-dictation"
        aria-labelledby="dictation-title"
      >
        <div className="workspace-task-icon">
          <Mic size={22} aria-hidden="true" />
        </div>
        <div className="min-w-0 flex-1">
          <h2 id="dictation-title" className="font-semibold">
            {t("workspace.dictate")}
          </h2>
          <p className="text-sm text-text/70 mt-1">
            {t("workspace.dictateHint")}
          </p>
          {shortcut ? (
            <kbd className="workspace-shortcut">{shortcut}</kbd>
          ) : (
            <p className="text-sm mt-2">{t("workspace.shortcutMissing")}</p>
          )}
          <button
            className="workspace-text-action"
            onClick={openDictationSettings}
          >
            {t("workspace.setupDictation")}
            <ArrowRight size={16} aria-hidden="true" />
          </button>
        </div>
      </section>
      <div className="workspace-tasks">
        {tasks.map(({ id, icon: Icon, key }) => (
          <button
            key={id}
            className="workspace-task"
            onClick={() => onNavigate(id)}
          >
            <Icon size={22} aria-hidden="true" />
            <span className="min-w-0 flex-1">
              <span className="block font-medium">{t(`workspace.${key}`)}</span>
              <span className="block text-sm text-text/70 mt-1">
                {t(`workspace.${key}Hint`)}
              </span>
            </span>
            <ArrowRight size={18} aria-hidden="true" />
          </button>
        ))}
      </div>
      <section aria-labelledby="workspace-tools">
        <h2 id="workspace-tools" className="text-sm font-medium mb-2">
          {t("workspace.tools")}
        </h2>
        <div className="workspace-tools">
          {(
            [
              { id: "models", icon: Cpu },
              { id: "settings", icon: Settings },
            ] as const
          ).map(({ id, icon: Icon }) => (
            <button
              key={id}
              className="workspace-task"
              onClick={() => onNavigate(id)}
            >
              <Icon size={20} aria-hidden="true" />
              <span>
                <span className="block text-sm font-medium">
                  {t(`sidebar.${id}`)}
                </span>
                <span className="block text-sm text-text/70">
                  {t(`workspace.${id}Hint`)}
                </span>
              </span>
            </button>
          ))}
        </div>
      </section>
    </div>
  );
}
