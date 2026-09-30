import React from "react";
import { useTranslation } from "react-i18next";
import { Upload } from "lucide-react";
import { IconAction } from "../../ui/IconAction";

/**
 * Symbol "Datei importieren" der Aufnahmezeile. Es fuehrt wie die Ablage auf
 * der Arbeitsflaeche in den einen Importweg (`useMeetingImport`): Datei waehlen,
 * Einwilligung, Import in das links gewaehlte Projekt.
 */
export const MeetingImportAction: React.FC<{
  busy: boolean;
  onPick: () => void;
}> = ({ busy, onPick }) => {
  const { t } = useTranslation();
  return (
    <IconAction
      icon={Upload}
      label={t("meetings.importAction.name")}
      description={t("meetings.importAction.hint")}
      testId="import-open"
      disabled={busy}
      onClick={onPick}
    />
  );
};
