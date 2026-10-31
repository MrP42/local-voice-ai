import React from "react";
import { useTranslation } from "react-i18next";
import { Info } from "lucide-react";
import type { SecretStoreState } from "@/lib/platform/secretStore";

/**
 * Hinweis vor jeder Eingabe von Zugangsdaten, wenn das System sie (noch)
 * nicht sicher speichern kann. Erscheint, bevor jemand etwas eintippt.
 */
export const SecretStoreNotice: React.FC<{
  state: Extract<SecretStoreState, { available: false }>;
}> = ({ state }) => {
  const { t } = useTranslation();
  return (
    <div
      role="note"
      data-testid="secret-store-notice"
      className="flex items-start gap-2 rounded-lg border border-mid-gray/30 bg-mid-gray/10 p-3 text-sm"
    >
      <Info size={16} aria-hidden="true" className="mt-0.5 shrink-0" />
      <p>{t(`integrations.secretStore.unavailable.${state.os}`)}</p>
    </div>
  );
};
