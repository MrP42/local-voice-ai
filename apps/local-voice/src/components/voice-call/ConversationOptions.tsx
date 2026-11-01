import React from "react";
import { useTranslation } from "react-i18next";
import {
  Menu,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
} from "@fluentui/react-components";
import { BookOpen, MoreHorizontal, Settings, SquarePen } from "lucide-react";
import { useVoiceCallStore } from "@/stores/voiceCallStore";

/**
 * Kontextmenü rechts oben im Modul („…“). Es ersetzt das Rad in der Kopfleiste: öffnet die rechte
 * Spalte (Gesprächseinstellungen, Reiter Wissen) und legt ein neues Gespräch an.
 */
export const ConversationOptions: React.FC = () => {
  const { t } = useTranslation();
  const openSettings = useVoiceCallStore((s) => s.openSettings);
  const newConversation = useVoiceCallStore((s) => s.newConversation);
  return (
    <Menu>
      <MenuTrigger disableButtonEnhancement>
        <button
          type="button"
          aria-label={t("voiceCall.options.label")}
          title={t("voiceCall.options.label")}
          className="grid place-items-center w-8 h-8 shrink-0 rounded cursor-pointer text-text/70 hover:bg-mid-gray/15 hover:text-text"
        >
          <MoreHorizontal className="w-4 h-4" aria-hidden="true" />
        </button>
      </MenuTrigger>
      <MenuPopover>
        <MenuList>
          <MenuItem
            icon={<Settings size={16} aria-hidden="true" />}
            onClick={() => openSettings()}
          >
            {t("voiceCall.settings.title")}
          </MenuItem>
          <MenuItem
            icon={<BookOpen size={16} aria-hidden="true" />}
            onClick={() => openSettings("knowledge")}
          >
            {t("voiceCall.options.knowledge")}
          </MenuItem>
          <MenuItem
            icon={<SquarePen size={16} aria-hidden="true" />}
            onClick={() => void newConversation()}
          >
            {t("voiceCall.newConversation")}
          </MenuItem>
        </MenuList>
      </MenuPopover>
    </Menu>
  );
};
