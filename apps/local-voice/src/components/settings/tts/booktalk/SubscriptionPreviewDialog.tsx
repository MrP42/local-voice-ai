import { useTranslation } from "react-i18next";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";

/**
 * Risikohinweis der Abo-Vorschau (lva#334): Claude Code und Codex lassen sich
 * vorab nicht auf ihren Tokenverbrauch beweisen. Erst nach „Verstanden,
 * einschalten“ wird der Schalter gesetzt und das Tupel (Anbieter, Modell,
 * CLI-Version, Prüfsumme) gebunden. Dieselbe Bestätigung gilt für „erneut
 * bestätigen“ nach einem CLI-Update.
 */
export function SubscriptionPreviewDialog({
  open,
  reconfirm,
  saving,
  error,
  onConfirm,
  onClose,
}: {
  open: boolean;
  /** Die Freigabe war gebunden und ist durch eine Änderung (CLI-Update) ungültig. */
  reconfirm: boolean;
  saving: boolean;
  /** Fehlercode des letzten Versuchs (`no_cli`, …) oder `null`. */
  error: string | null;
  onConfirm(): void;
  onClose(): void;
}) {
  const { t } = useTranslation();
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && onClose()}
      title={t("tts.bookTalk.subscription.title")}
      closeLabel={t("tts.bookTalk.scope.confirmClose")}
      footer={
        <>
          <Button
            variant="primary"
            disabled={saving}
            onClick={onConfirm}
            data-testid="booktalk-subscription-confirm"
          >
            {t("tts.bookTalk.subscription.confirm")}
          </Button>
          <Button variant="secondary" onClick={onClose}>
            {t("tts.bookTalk.subscription.cancel")}
          </Button>
        </>
      }
    >
      <div data-testid="booktalk-subscription-dialog">
        {reconfirm && (
          <p className="text-sm" data-testid="booktalk-subscription-reconfirm">
            {t("tts.bookTalk.subscription.reconfirm")}
          </p>
        )}
        <p className="text-sm">{t("tts.bookTalk.subscription.risk")}</p>
        <p className="text-sm">{t("tts.bookTalk.subscription.cloud")}</p>
        {error && (
          <p role="alert" className="text-sm" data-testid="booktalk-subscription-error">
            {t(`tts.bookTalk.subscription.error.${error === "no_cli" ? "noCli" : "other"}`)}
          </p>
        )}
      </div>
    </Dialog>
  );
}
