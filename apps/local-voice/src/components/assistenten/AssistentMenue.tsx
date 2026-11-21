import React from "react";
import { useTranslation } from "react-i18next";
import {
  Menu,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
} from "@fluentui/react-components";
import { Copy, Download, Ellipsis, Pencil, Trash2 } from "lucide-react";

interface Props {
  /** Name für die Beschriftung des Knopfes („Aktionen für …“). */
  name: string;
  art: "vorlage" | "eigen";
  /** Vorgänge, die schreiben, sind gesperrt (Speicher nicht lesbar, ein Vorgang läuft). */
  schreibt: boolean;
  onKopieren: () => void;
  onExportieren: () => void;
  onUmbenennen?: () => void;
  onLoeschen?: () => void;
  testId?: string;
  /** Ziele auf dem Telefon mindestens 44 px. */
  className?: string;
}

/**
 * Das Menü der Aktionen eines Assistenten, in der Liste und in der Ansicht dasselbe: bei einer Vorlage „Als eigenen kopieren“
 * und „Exportieren“, bei einem eigenen Assistenten außerdem „Umbenennen“ und „Löschen“. Tastatur und Screenreader wie bei
 * jedem Fluent-Menü (Pfeiltasten, Esc).
 */
export const AssistentMenue: React.FC<Props> = ({
  name,
  art,
  schreibt,
  onKopieren,
  onExportieren,
  onUmbenennen,
  onLoeschen,
  testId,
  className = "lv-asst-zeile__menue",
}) => {
  const { t } = useTranslation();
  const eintrag = "max-sm:!min-h-[44px]";
  return (
    <Menu>
      <MenuTrigger disableButtonEnhancement>
        <button
          type="button"
          className={className}
          aria-label={t("assistenten.liste.aktionen", { name })}
          data-testid={testId}
        >
          <Ellipsis size={18} aria-hidden="true" />
        </button>
      </MenuTrigger>
      <MenuPopover>
        <MenuList>
          <MenuItem
            icon={<Copy size={16} aria-hidden="true" />}
            disabled={!schreibt}
            onClick={onKopieren}
            className={eintrag}
            data-testid="assistent-aktion-kopieren"
          >
            {art === "vorlage"
              ? t("assistenten.aktion.alsEigenen")
              : t("assistenten.aktion.kopieren")}
          </MenuItem>
          {onUmbenennen && (
            <MenuItem
              icon={<Pencil size={16} aria-hidden="true" />}
              disabled={!schreibt}
              onClick={onUmbenennen}
              className={eintrag}
              data-testid="assistent-aktion-umbenennen"
            >
              {t("assistenten.aktion.umbenennen")}
            </MenuItem>
          )}
          <MenuItem
            icon={<Download size={16} aria-hidden="true" />}
            onClick={onExportieren}
            className={eintrag}
            data-testid="assistent-aktion-exportieren"
          >
            {t("assistenten.aktion.exportieren")}
          </MenuItem>
          {onLoeschen && (
            <MenuItem
              icon={<Trash2 size={16} aria-hidden="true" />}
              disabled={!schreibt}
              onClick={onLoeschen}
              className={eintrag}
              data-testid="assistent-aktion-loeschen"
            >
              {t("assistenten.aktion.loeschen")}
            </MenuItem>
          )}
        </MenuList>
      </MenuPopover>
    </Menu>
  );
};
