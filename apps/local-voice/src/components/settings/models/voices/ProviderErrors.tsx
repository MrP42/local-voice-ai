import React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../../ui/Button";
import type { TtsProvidersState } from "../../tts/providers/useTtsProviders";

/**
 * Lade- und Aktionsfehler der Anbieterliste (role="alert"), gemeinsam fuer
 * Anbieteransicht, Stimmenansicht und Seitenblatt. Bei einem Ladefehler gibt es
 * "Erneut versuchen".
 */
export const ProviderErrors: React.FC<{
  providers: Pick<
    TtsProvidersState,
    "loadError" | "actionError" | "reload" | "checking"
  >;
  /** Praefix der Testkennungen; das Seitenblatt nutzt ein eigenes. */
  testIdPrefix?: string;
}> = ({ providers, testIdPrefix = "tts-providers" }) => {
  const { t } = useTranslation();
  const { loadError, actionError, checking } = providers;
  if (!loadError && !actionError) return null;
  return (
    <div className="space-y-1">
      {loadError && (
        <div className="flex flex-wrap items-center gap-2">
          <p
            role="alert"
            className="text-xs text-red-500"
            data-testid={`${testIdPrefix}-load-error`}
          >
            {t("tts.providers.loadError", { message: loadError })}
          </p>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            className="min-h-[44px] md:min-h-8"
            disabled={checking}
            data-testid={`${testIdPrefix}-load-retry`}
            onClick={() => void providers.reload()}
          >
            {t("settings.models.voiceManager.providers.retry")}
          </Button>
        </div>
      )}
      {actionError && (
        <p
          role="alert"
          className="text-xs text-red-500"
          data-testid={`${testIdPrefix}-action-error`}
        >
          {actionError}
        </p>
      )}
    </div>
  );
};
