import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { useSettings } from "../../../../hooks/useSettings";
import { modelLabel } from "@/lib/modelNames";
import {
  assessedById,
  jurisdictionOf,
} from "@/components/voice-call/orchestration";
import {
  bookModelOf,
  estimateAskCostUsd,
  providerIdOf,
  withConsent,
} from "./consent";

/**
 * Freigabe für ein Cloud-Modell (lva#309, Plan §3.8): Anbieter, Modell, Region,
 * was gesendet wird, geschätzte Kosten je Frage. Erst nach „Freigeben“ wird
 * gespeichert (`book_talk_cloud_consent`) und die Frage noch einmal gestellt.
 */
export function ConsentDialog({
  open,
  onClose,
  onGranted,
}: {
  open: boolean;
  onClose(): void;
  /** Freigabe gespeichert: die Frage kann neu gestellt werden. */
  onGranted(): void;
}) {
  const { t, i18n } = useTranslation();
  const { settings, updateSetting } = useSettings();
  const [region, setRegion] = useState<"eu" | "other" | "unknown">("unknown");
  const [saving, setSaving] = useState(false);
  const found = bookModelOf(settings);

  useEffect(() => {
    if (!open || !found) return;
    let alive = true;
    void commands
      .complianceAssessModels()
      .then((list) => {
        if (!alive || !list) return;
        const assessment = assessedById(list)[found.model.id];
        setRegion(assessment ? jurisdictionOf(assessment) : "unknown");
      })
      .catch(() => alive && setRegion("unknown"));
    return () => {
      alive = false;
    };
  }, [open, found?.model.id]);

  if (!found) return null;
  const provider = found.connection.label || found.connection.kind;
  const cost = estimateAskCostUsd(found.model);
  const costText =
    cost === null
      ? t("tts.bookTalk.consent.costUnknown")
      : t("tts.bookTalk.consent.cost", {
          cost: cost.toLocaleString(i18n.language, {
            minimumFractionDigits: 3,
            maximumFractionDigits: 4,
          }),
        });

  const allow = async () => {
    setSaving(true);
    try {
      await updateSetting(
        "book_talk_cloud_consent",
        withConsent(settings?.book_talk_cloud_consent, providerIdOf(found.connection)),
      );
      onGranted();
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && onClose()}
      title={t("tts.bookTalk.consent.title", { provider })}
      closeLabel={t("tts.bookTalk.scope.confirmClose")}
      className="bt-consent"
      footer={
        <>
          <Button
            variant="primary"
            disabled={saving}
            onClick={() => void allow()}
            data-testid="booktalk-consent-allow"
          >
            {t("tts.bookTalk.consent.allow")}
          </Button>
          <Button variant="secondary" onClick={onClose}>
            {t("tts.bookTalk.consent.deny")}
          </Button>
        </>
      }
    >
      <div className="bt-consent" data-testid="booktalk-consent">
        <dl>
          <dt>{t("tts.bookTalk.consent.provider")}</dt>
          <dd data-testid="booktalk-consent-provider">{provider}</dd>
          <dt>{t("tts.bookTalk.consent.model")}</dt>
          <dd data-testid="booktalk-consent-model">
            {modelLabel(found.model.remote_id, found.model.label)}
          </dd>
          <dt>{t("tts.bookTalk.consent.region")}</dt>
          <dd>
            {t(
              region === "eu"
                ? "tts.bookTalk.consent.regionEu"
                : region === "other"
                  ? "tts.bookTalk.consent.regionOther"
                  : "tts.bookTalk.consent.regionUnknown",
            )}
          </dd>
        </dl>
        <p>{t("tts.bookTalk.consent.what")}</p>
        <p data-testid="booktalk-consent-cost">{costText}</p>
        <p className="text-text-muted">{t("tts.bookTalk.consent.revoke")}</p>
      </div>
    </Dialog>
  );
}
