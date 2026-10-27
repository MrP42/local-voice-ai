import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Checkbox } from "@fluentui/react-components";
import { commands, type TtsProviderEntry } from "@/bindings";
import { useSettings } from "../../../../hooks/useSettings";
import { SettingsGroup } from "../../../ui/SettingsGroup";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { ProviderDetails } from "./ProviderDetails";
import { ProviderRow, VerifiedLine } from "./ProviderRow";
import { asTtsError, nextApprovalStep } from "./providerModel";
import { useProviderPreview } from "./useProviderPreview";

type DialogState = { kind: "license" | "remote"; id: string } | null;

/**
 * Gruppe "Sprachausgabe-Anbieter" im Reiter Ausgabe: alle Anbieter mit
 * Faehigkeiten, Zustand und Lizenz, der Freigabe-Ablauf (Aktivieren, Lizenz
 * bestaetigen, Fernzugriff freigeben), Einstellungen, Stimmenwahl und
 * Probestimme. Die Oberflaeche liest nur, was die Befehle liefern -- neue
 * Anbieter erscheinen ohne Aenderung hier.
 */
export const TtsProvidersGroup: React.FC = () => {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<TtsProviderEntry[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [dialog, setDialog] = useState<DialogState>(null);
  const preview = useProviderPreview();
  const { refreshSettings } = useSettings();

  const reload = useCallback(async (): Promise<TtsProviderEntry[] | null> => {
    setChecking(true);
    try {
      const result = await commands.ttsProvidersList();
      if (result.status === "ok") {
        setEntries(result.data);
        setLoadError(null);
        return result.data;
      }
      setLoadError(result.error.message);
    } catch (e) {
      setLoadError(asTtsError(e).message);
    } finally {
      setChecking(false);
    }
    return null;
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  /** Nach einem Schritt der Freigabe: der naechste Dialog, falls noch einer faellig ist. */
  const advance = useCallback((id: string, list: TtsProviderEntry[] | null) => {
    const entry = list?.find((e) => e.descriptor.id === id);
    if (!entry) return;
    const step = nextApprovalStep(entry);
    if (step) setDialog({ kind: step, id });
  }, []);

  const run = async (
    id: string,
    action: () => Promise<
      { status: "ok" } | { status: "error"; error: unknown }
    >,
  ): Promise<boolean> => {
    setBusy(id);
    setActionError(null);
    try {
      const result = await action();
      if (result.status === "error") {
        setActionError(asTtsError(result.error).message);
        return false;
      }
      return true;
    } catch (e) {
      setActionError(asTtsError(e).message);
      return false;
    } finally {
      setBusy(null);
    }
  };

  const enable = async (id: string, enabled: boolean) => {
    const ok = await run(id, () => commands.ttsProviderSetEnabled(id, enabled));
    const list = await reload();
    if (ok && enabled) advance(id, list);
  };

  const recheck = async (id: string) => {
    setBusy(id);
    try {
      const result = await commands.ttsProviderHealth(id);
      if (result.status === "ok") {
        setEntries((cur) =>
          (cur ?? []).map((e) =>
            e.descriptor.id === id ? { ...e, health: result.data } : e,
          ),
        );
      } else setActionError(result.error.message);
    } catch (e) {
      setActionError(asTtsError(e).message);
    } finally {
      setBusy(null);
    }
  };

  const toggleExpanded = (id: string) =>
    setExpanded((cur) => {
      const next = new Set(cur);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const dialogEntry =
    dialog && entries
      ? (entries.find((e) => e.descriptor.id === dialog.id) ?? null)
      : null;

  return (
    <SettingsGroup
      title={t("tts.providers.title")}
      description={t("tts.providers.description")}
      anchor="tts-providers"
    >
      <div className="flex flex-wrap items-center gap-3 px-4 py-3">
        <Button
          type="button"
          variant="secondary"
          size="sm"
          disabled={checking}
          data-testid="tts-providers-recheck-all"
          onClick={() => void reload()}
        >
          {t("tts.providers.recheckAll")}
        </Button>
        {checking && (
          <span
            role="status"
            className="text-xs text-text/70"
            data-testid="tts-providers-checking"
          >
            {t("tts.providers.checking")}
          </span>
        )}
      </div>
      {loadError && (
        <p
          role="alert"
          className="px-4 py-3 text-xs text-red-500"
          data-testid="tts-providers-load-error"
        >
          {t("tts.providers.loadError", { message: loadError })}
        </p>
      )}
      {actionError && (
        <p
          role="alert"
          className="px-4 py-3 text-xs text-red-500"
          data-testid="tts-providers-action-error"
        >
          {actionError}
        </p>
      )}
      {entries?.map((entry) => {
        const id = entry.descriptor.id;
        return (
          <ProviderRow
            key={id}
            entry={entry}
            expanded={expanded.has(id)}
            busy={busy === id}
            onToggleExpanded={() => toggleExpanded(id)}
            onEnable={(on) => void enable(id, on)}
            onRecheck={() => void recheck(id)}
            onApprove={(kind) => setDialog({ kind, id })}
            onRevokeRemote={() =>
              void run(id, () =>
                commands.ttsProviderSetAllowRemote(id, false),
              ).then(() => reload())
            }
          >
            <ProviderDetails
              entry={entry}
              preview={preview.state}
              onPreview={(text, voice) => void preview.start(id, text, voice)}
              onCancelPreview={() => void preview.cancel()}
              onChanged={() => {
                void reload();
                // Eine neue Vorlese-Stimme steht auch im Dropdown der Vorlesen-Seite.
                void refreshSettings();
              }}
            />
          </ProviderRow>
        );
      })}
      {entries && entries.length === 0 && (
        <p className="px-4 py-3 text-xs text-text/70">
          {t("tts.providers.empty")}
        </p>
      )}
      {dialog?.kind === "license" && dialogEntry && (
        <LicenseDialog
          entry={dialogEntry}
          onClose={() => setDialog(null)}
          onAcknowledged={async () => {
            const list = await reload();
            setDialog(null);
            advance(dialogEntry.descriptor.id, list);
          }}
          onStale={() => void reload()}
        />
      )}
      {dialog?.kind === "remote" && dialogEntry && (
        <RemoteDialog
          entry={dialogEntry}
          onClose={() => setDialog(null)}
          onAllowed={async () => {
            await reload();
            setDialog(null);
          }}
        />
      )}
    </SettingsGroup>
  );
};

// ---------------------------------------------------------------------------

const LicenseDialog: React.FC<{
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

const RemoteDialog: React.FC<{
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

export default TtsProvidersGroup;
