import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Checkbox } from "@fluentui/react-components";
import { commands, type TtsProviderEntry } from "@/bindings";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { VerifiedLine } from "./ProviderRow";
import { asTtsError } from "./providerModel";

/** Freigabe-Dialoge der Anbieter: Lizenz bestaetigen und Fernzugriff freigeben. */

export const LicenseDialog: React.FC<{
  entry: TtsProviderEntry;
  onClose: () => void;
  onAcknowledged: () => Promise<void>;
  onStale: () => void;
}> = ({ entry, onClose, onAcknowledged, onStale }) => {
  const { t } = useTranslation();
  const { license, label } = entry.descriptor;
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [working, setWorking] = useState(false);

  // Der Fingerabdruck hat sich geaendert (Modell/Lizenzangabe): die frühere
  // Bestätigung gilt nicht für den neuen Stand.
  useEffect(() => {
    setConfirmed(false);
  }, [entry.fingerprint]);

  const confirm = async () => {
    setWorking(true);
    setError(null);
    try {
      const result = await commands.ttsProviderAcknowledgeLicense(
        entry.descriptor.id,
        entry.fingerprint,
      );
      if (result.status === "ok") {
        await onAcknowledged();
      } else {
        setError(result.error.message);
        // bad_request: der Stand hat sich geaendert -- Liste neu laden, der
        // Dialog zeigt dann den neuen Stand und der Nutzer bestaetigt erneut.
        if (result.error.kind === "bad_request") onStale();
        setConfirmed(false);
      }
    } catch (e) {
      setError(asTtsError(e).message);
    } finally {
      setWorking(false);
    }
  };

  return (
    <Dialog
      open
      title={t("tts.providers.licenseDialog.title", { name: label })}
      description={t("tts.providers.licenseDialog.intro")}
      closeLabel={t("tts.providers.dialog.close")}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      className="max-w-lg"
      footer={
        <>
          <Button
            type="button"
            variant="primary"
            disabled={!confirmed || working}
            data-testid="tts-provider-license-confirm"
            onClick={() => void confirm()}
          >
            {t("tts.providers.licenseDialog.confirm")}
          </Button>
          <Button type="button" variant="secondary" onClick={onClose}>
            {t("tts.providers.dialog.cancel")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="tts-provider-license-dialog">
        <dl className="grid grid-cols-[max-content_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm">
          <dt className="text-mid-gray">{t("tts.providers.license.code")}</dt>
          <dd>{license.code || "–"}</dd>
          <dt className="text-mid-gray">
            {t("tts.providers.license.weights")}
          </dt>
          <dd>{license.weights || "–"}</dd>
          <dt className="text-mid-gray">{t("tts.providers.facts.license")}</dt>
          <dd>
            <VerifiedLine license={license} />
          </dd>
        </dl>
        {license.non_commercial && (
          <p className="text-sm text-amber-700 dark:text-amber-400">
            {t("tts.providers.licenseDialog.nonCommercial")}
          </p>
        )}
        {!license.verified && (
          <p className="text-sm text-amber-700 dark:text-amber-400">
            {t("tts.providers.licenseDialog.notVerified")}
          </p>
        )}
        {license.note && <p className="text-sm text-text/80">{license.note}</p>}
        <Checkbox
          checked={confirmed}
          onChange={(_e, data) => setConfirmed(data.checked === true)}
          label={t("tts.providers.licenseDialog.checkbox", { name: label })}
          data-testid="tts-provider-license-checkbox"
        />
        {error && (
          <p role="alert" className="text-sm text-red-500">
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
};

export const RemoteDialog: React.FC<{
  entry: TtsProviderEntry;
  onClose: () => void;
  onAllowed: () => Promise<void>;
}> = ({ entry, onClose, onAllowed }) => {
  const { t } = useTranslation();
  const [error, setError] = useState<string | null>(null);
  const [working, setWorking] = useState(false);
  const { label, id } = entry.descriptor;

  const allow = async () => {
    setWorking(true);
    setError(null);
    try {
      const result = await commands.ttsProviderSetAllowRemote(id, true);
      if (result.status === "ok") await onAllowed();
      else setError(result.error.message);
    } catch (e) {
      setError(asTtsError(e).message);
    } finally {
      setWorking(false);
    }
  };

  return (
    <Dialog
      open
      title={t("tts.providers.remoteDialog.title", { name: label })}
      closeLabel={t("tts.providers.dialog.close")}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      className="max-w-lg"
      footer={
        <>
          <Button
            type="button"
            variant="primary"
            disabled={working}
            data-testid="tts-provider-remote-confirm"
            onClick={() => void allow()}
          >
            {t("tts.providers.remoteDialog.confirm")}
          </Button>
          <Button type="button" variant="secondary" onClick={onClose}>
            {t("tts.providers.dialog.cancel")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="tts-provider-remote-dialog">
        <p role="alert" className="text-sm text-amber-700 dark:text-amber-400">
          {t("tts.providers.remoteDialog.warning", { name: label })}
        </p>
        <p className="text-sm text-text/80">
          {t("tts.providers.remoteDialog.detail")}
        </p>
        {error && (
          <p role="alert" className="text-sm text-red-500">
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
};
