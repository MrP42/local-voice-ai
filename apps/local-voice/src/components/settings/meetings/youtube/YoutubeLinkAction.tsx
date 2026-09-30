import React from "react";
import { useTranslation } from "react-i18next";
import { Link } from "lucide-react";
import { IconAction } from "../../../ui/IconAction";
import { openYoutubeLinkDialog } from "./linkBus";

/**
 * Symbol "Link einfuegen" der Aufnahmezeile (neben "Datei importieren"). Es
 * oeffnet nur den Dialog; der sitzt einmal auf der Seite (`YoutubeLinkHost`).
 */
export const YoutubeLinkAction: React.FC = () => {
  const { t } = useTranslation();
  return (
    <IconAction
      icon={Link}
      label={t("meetings.importAction.linkName")}
      description={t("meetings.importAction.linkHint")}
      testId="link-open"
      onClick={() => openYoutubeLinkDialog()}
    />
  );
};
