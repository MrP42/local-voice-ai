import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  openLanguageModels,
  openLanguageModelConnections,
} from "@/lib/llmSetup";

export function LanguageModelSetupHint({
  connections = true,
  beforeNavigate,
}: {
  connections?: boolean;
  beforeNavigate?: () => Promise<boolean>;
}) {
  const { t } = useTranslation();
  const [opening, setOpening] = useState(false);
  const navigate = async (open: () => void) => {
    if (opening) return;
    setOpening(true);
    try {
      if (!beforeNavigate || (await beforeNavigate())) open();
    } finally {
      setOpening(false);
    }
  };
  return (
    <div
      className="space-y-1 text-sm text-text/70"
      data-testid="language-model-setup"
    >
      <p>{t("modelSetup.description")}</p>
      <div className="flex flex-wrap gap-x-3 gap-y-1">
        <button
          type="button"
          disabled={opening}
          onClick={() => void navigate(openLanguageModels)}
          className="text-text underline underline-offset-4 hover:text-logo-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary rounded-sm"
        >
          {t("modelSetup.choose")}
        </button>
        {connections && (
          <button
            type="button"
            disabled={opening}
            onClick={() => void navigate(openLanguageModelConnections)}
            className="underline underline-offset-4 hover:text-logo-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary rounded-sm"
          >
            {t("modelSetup.connect")}
          </button>
        )}
      </div>
    </div>
  );
}
