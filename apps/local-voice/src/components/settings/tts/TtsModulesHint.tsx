import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/Button";

export function TtsModulesHint() {
  const { t } = useTranslation();
  return (
    <div
      className="rounded-lg border border-mid-gray/30 p-3 space-y-2"
      data-testid="tts-modules-hint"
    >
      <p className="text-sm text-text/70">{t("tts.modules.optional")}</p>
      <Button
        variant="secondary"
        size="sm"
        onClick={() => {
          window.dispatchEvent(
            new CustomEvent("lv-navigate", { detail: { section: "models" } }),
          );
        }}
      >
        {t("tts.modules.manage")}
      </Button>
    </div>
  );
}
