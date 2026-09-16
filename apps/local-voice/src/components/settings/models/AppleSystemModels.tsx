import { useEffect, useState } from "react";
import { platform } from "@tauri-apps/plugin-os";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "@/components/ui/Button";

export function AppleSystemModels() {
  const { t } = useTranslation();
  const { getSetting, refreshSettings } = useSettings();
  const [status, setStatus] = useState<Awaited<
    ReturnType<typeof commands.appleSystemStatus>
  > | null>(null);
  const [error, setError] = useState<string | null>(null);
  const isMac = platform() === "macos";
  useEffect(() => {
    if (!isMac) return;
    let live = true;
    void commands
      .appleSystemStatus()
      .then((value) => {
        if (live) setStatus(value);
      })
      .catch((error: unknown) => {
        if (live) setError(String(error));
      });
    return () => {
      live = false;
    };
  }, [isMac]);
  if (!isMac) return null;
  const active =
    getSetting("llm_active_model_id") ===
    "apple_intelligence:Apple Intelligence";
  return (
    <div
      className="rounded-lg border border-mid-gray/30 p-4 space-y-2"
      data-testid="apple-system-models"
    >
      <h3 className="text-sm font-semibold">{t("appleSystem.title")}</h3>
      <p className="text-sm text-text/70">{t("appleSystem.voices")}</p>
      {status && (
        <>
          <p className="text-sm text-text/70">
            {t(
              status.speech_available
                ? "appleSystem.speechReady"
                : "appleSystem.speechUnavailable",
            )}
          </p>
          <p className="text-sm text-text/70">
            {t(
              status.llm_available
                ? "appleSystem.llmReady"
                : "appleSystem.llmUnavailable",
            )}
          </p>
          {status.llm_available && (
            <Button
              size="sm"
              variant="secondary"
              disabled={active}
              onClick={async () => {
                setError(null);
                try {
                  await commands.appleSystemInitialize();
                  const result = await commands.llmSetActiveModel(
                    "apple_intelligence:Apple Intelligence",
                  );
                  if (result.status === "error") throw new Error(result.error);
                  await refreshSettings();
                } catch (error) {
                  setError(String(error));
                }
              }}
            >
              {t(active ? "appleSystem.active" : "appleSystem.useLLM")}
            </Button>
          )}
        </>
      )}
      {error && (
        <p className="text-xs text-red-500" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
