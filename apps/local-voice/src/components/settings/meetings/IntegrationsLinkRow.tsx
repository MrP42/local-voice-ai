import React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../ui/Button";
import { SettingContainer } from "../../ui/SettingContainer";

interface IntegrationsLinkRowProps {
  /** Welcher Umzug: bestimmt Titel, Text und Kennung. */
  topic: "calendar" | "mcp";
}

/**
 * Verweis auf die Seite „Integrationen“ (A4, E6): Kalenderquellen und der
 * MCP-Schalter wohnen dort; unter Einstellungen > Besprechungen bleibt nur
 * diese Zeile, damit man sie beim Suchen am alten Ort noch findet. Der Sprung
 * nutzt dasselbe Ereignis wie die anderen Querverweise (`lv-navigate`).
 */
export const IntegrationsLinkRow: React.FC<IntegrationsLinkRowProps> = ({
  topic,
}) => {
  const { t } = useTranslation();
  const go = () =>
    window.dispatchEvent(
      new CustomEvent("lv-navigate", { detail: { section: "integrations" } }),
    );
  return (
    <SettingContainer
      title={t(`integrations.moved.${topic}.title`)}
      description={t(`integrations.moved.${topic}.description`)}
      grouped={true}
      descriptionMode="tooltip"
    >
      <Button
        variant="secondary"
        size="sm"
        onClick={go}
        data-testid={`${topic}-settings-link`}
      >
        {t("integrations.moved.open")}
      </Button>
    </SettingContainer>
  );
};
