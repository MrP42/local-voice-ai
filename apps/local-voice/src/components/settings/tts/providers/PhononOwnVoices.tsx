import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import {
  commands,
  type KeyStatus,
  type OwnVoice,
  type OwnVoicesOverview,
  type VoiceInfo,
} from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";
import { PhononCloneDialog } from "./PhononCloneDialog";
import { PhononImportDialog } from "./PhononImportDialog";
import { asPhononError, isCloned, modelLabel } from "./phononVoicesModel";
import { SamplePlayButton } from "./SamplePlayButton";
import { useVoiceSamples } from "./useVoiceSamples";

/** Die Stimmenlisten (Vorlesen-Dropdown, Auswahl „im Vorlesen anzeigen“) laden neu. */
const announceVoicesChanged = () =>
  window.dispatchEvent(new CustomEvent("lv-voices-changed"));

/**
 * Eigene Phonon-Stimmen in den Phonon-Einstellungen: importieren, mit Gradium
 * erstellen (Datei, Mikrofon, vorhandene Fish-Stimme), entfernen, und der
 * Gradium-Schlüssel. Die Stimmen liegen danach lokal im Phonon-Modell und
 * werden ohne Netz benutzt; nur „Stimme erstellen“ geht ins Netz, nach Hinweis
 * und Einwilligung (PhononCloneDialog).
 */
export const PhononOwnVoices: React.FC = () => {
  const { t } = useTranslation();
  const samples = useVoiceSamples("phonon");
  const [overview, setOverview] = useState<OwnVoicesOverview | null>(null);
  const [keyStatus, setKeyStatus] = useState<KeyStatus | null>(null);
  const [fish, setFish] = useState<VoiceInfo[]>([]);
  const [importPath, setImportPath] = useState<string | null>(null);
  const [clone, setClone] = useState<{ fishId: string | null } | null>(null);
  const [removing, setRemoving] = useState<OwnVoice | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const reload = useCallback(async () => {
    const [own, key, fishVoices] = await Promise.all([
      commands.phononOwnVoicesList().catch(() => null),
      commands.phononGradiumKeyStatus().catch(() => null),
      commands.ttsListVoiceInfos().catch(() => [] as VoiceInfo[]),
    ]);
    if (own?.status === "ok" && own.data) setOverview(own.data);
    if (key?.status === "ok" && key.data) setKeyStatus(key.data);
    setFish(Array.isArray(fishVoices) ? fishVoices : []);
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const changed = useCallback(
    (message: string | null) => {
      setNotice(message);
      setError(null);
      announceVoicesChanged();
      void reload();
    },
    [reload],
  );

  const chooseImport = async () => {
    setError(null);
    try {
      const picked = await openFile({
        multiple: false,
        filters: [
          {
            name: t("tts.providers.phononVoices.import.filter"),
            extensions: ["safetensors"],
          },
        ],
      });
      if (typeof picked === "string") setImportPath(picked);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const remove = async (voice: OwnVoice) => {
    const result = await commands
      .phononOwnVoiceRemove(voice.id)
      .catch((e) => ({ status: "error" as const, error: asPhononError(e) }));
    setRemoving(null);
    if (result.status === "ok") {
      changed(t("tts.providers.phononVoices.removed", { name: voice.name }));
    } else {
      setError(result.error.message);
    }
  };

  const voices = overview?.voices ?? [];
  const installed = overview?.models ?? [];

  return (
    <section
      aria-label={t("tts.providers.phononVoices.title")}
      className="space-y-3"
      data-testid="phonon-own-voices"
    >
      <div className="space-y-1">
        <h3 className="text-xs font-medium text-mid-gray">
          {t("tts.providers.phononVoices.title")}
        </h3>
        <p className="text-xs text-text/70">
          {t("tts.providers.phononVoices.intro")}
        </p>
        {overview && installed.length === 0 && (
          <p
            className="text-xs text-yellow-500"
            data-testid="phonon-own-no-model"
          >
            {t("tts.providers.phononVoices.noModel")}
          </p>
        )}
      </div>

      {voices.length === 0 ? (
        <p className="text-xs text-text/70" data-testid="phonon-own-empty">
          {t("tts.providers.phononVoices.empty")}
        </p>
      ) : (
        <ul
          className="divide-y divide-mid-gray/15 rounded-md border border-mid-gray/20"
          data-testid="phonon-own-list"
        >
          {voices.map((voice) => (
            <li
              key={voice.id}
              className="flex items-start gap-2 px-3 py-2"
              data-testid={`phonon-own-voice-${voice.id}`}
            >
              <div className="min-w-0 flex-1 text-sm">
                <div className="flex flex-wrap items-baseline gap-x-2">
                  <span className="font-medium text-text">{voice.name}</span>
                  <span className="text-xs text-text/70">
                    {isCloned(voice)
                      ? t("tts.providers.phononVoices.originClone")
                      : t("tts.providers.phononVoices.originImport")}
                  </span>
                  {voice.language && (
                    <span className="text-xs text-text/70">
                      {voice.language}
                    </span>
                  )}
                </div>
                <p className="text-xs text-text/70">
                  {t("tts.providers.phononVoices.license")}
                  {" · "}
                  {voice.in_models.length > 0
                    ? t("tts.providers.phononVoices.inModels", {
                        models: voice.in_models.map(modelLabel).join(", "),
                      })
                    : t("tts.providers.phononVoices.inNoModel")}
                </p>
                {voice.source_note && (
                  <p className="text-xs text-text/70">{voice.source_note}</p>
                )}
              </div>
              {voice.id in samples && (
                <SamplePlayButton
                  path={samples[voice.id]}
                  name={voice.name}
                  testId={`phonon-own-sample-${voice.id}`}
                />
              )}
              <Button
                type="button"
                variant="danger-ghost"
                size="sm"
                data-testid={`phonon-own-remove-${voice.id}`}
                onClick={() => setRemoving(voice)}
              >
                {t("tts.providers.phononVoices.remove")}
              </Button>
            </li>
          ))}
        </ul>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="secondary"
          size="sm"
          data-testid="phonon-import-button"
          onClick={() => void chooseImport()}
        >
          {t("tts.providers.phononVoices.importButton")}
        </Button>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          data-testid="phonon-create-button"
          onClick={() => setClone({ fishId: null })}
        >
          {t("tts.providers.phononVoices.createButton")}
        </Button>
      </div>

      {notice && (
        <p
          role="status"
          className="text-xs text-text/80"
          data-testid="phonon-own-notice"
        >
          {notice}
        </p>
      )}
      {error && (
        <p
          role="alert"
          className="text-xs text-red-500"
          data-testid="phonon-own-error"
        >
          {error}
        </p>
      )}

      <GradiumKey status={keyStatus} onStatus={setKeyStatus} />

      <div className="space-y-1" data-testid="phonon-fish-voices">
        <h4 className="text-xs font-medium text-text/80">
          {t("tts.providers.phononVoices.fish.title")}
        </h4>
        <p className="text-xs text-text/70">
          {t("tts.providers.phononVoices.fish.hint")}
        </p>
        {fish.length === 0 ? (
          <p className="text-xs text-text/70">
            {t("tts.providers.phononVoices.fish.empty")}
          </p>
        ) : (
          <ul className="max-h-48 divide-y divide-mid-gray/15 overflow-y-auto rounded-md border border-mid-gray/20">
            {fish.map((v) => (
              <li
                key={v.id}
                className="flex items-center gap-2 px-3 py-1.5"
                data-testid={`phonon-fish-${v.id}`}
              >
                <span className="min-w-0 flex-1 truncate text-sm">
                  {v.meta.display_name || v.id}
                </span>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  data-testid={`phonon-fish-take-${v.id}`}
                  onClick={() => setClone({ fishId: v.id })}
                >
                  {t("tts.providers.phononVoices.fish.take")}
                </Button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <PhononImportDialog
        path={importPath}
        onClose={() => setImportPath(null)}
        onDone={(voice) => {
          setImportPath(null);
          changed(
            t("tts.providers.phononVoices.imported", { name: voice.name }),
          );
        }}
      />
      <PhononCloneDialog
        open={clone !== null}
        initialFishId={clone?.fishId ?? null}
        fishVoices={fish}
        languages={overview?.languages ?? []}
        keyStatus={keyStatus}
        onClose={() => setClone(null)}
        onDone={(voice) =>
          changed(t("tts.providers.phononVoices.created", { name: voice.name }))
        }
      />
      <Dialog
        open={removing !== null}
        onOpenChange={(open) => {
          if (!open) setRemoving(null);
        }}
        title={t("tts.providers.phononVoices.removeTitle", {
          name: removing?.name ?? "",
        })}
        description={t("tts.providers.phononVoices.removeText")}
        closeLabel={t("common.close")}
        footer={
          <>
            <Button
              type="button"
              variant="danger"
              data-testid="phonon-own-remove-confirm"
              onClick={() => removing && void remove(removing)}
            >
              {t("tts.providers.phononVoices.remove")}
            </Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => setRemoving(null)}
            >
              {t("common.cancel")}
            </Button>
          </>
        }
      >
        {removing && isCloned(removing) && (
          <p className="text-sm">
            {t("tts.providers.phononVoices.removeCloneNote")}
          </p>
        )}
      </Dialog>
    </section>
  );
};

// ---------------------------------------------------------------------------

/**
 * Gradium-Schlüssel eintragen, ändern, löschen. Er geht in den verschlüsselten
 * Geheimnisspeicher (nie in die Einstellungen) und kommt nie zurück: gezeigt
 * wird nur, OB einer da ist. Das Speichern sendet nichts.
 */
const GradiumKey: React.FC<{
  status: KeyStatus | null;
  onStatus: (s: KeyStatus) => void;
}> = ({ status, onStatus }) => {
  const { t } = useTranslation();
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const save = async () => {
    setBusy(true);
    setError(null);
    setSaved(false);
    const result = await commands
      .phononGradiumKeySet(value)
      .catch((e) => ({ status: "error" as const, error: asPhononError(e) }));
    setBusy(false);
    if (result.status === "ok") {
      setValue(""); // der Schlüssel bleibt nirgends in der Oberfläche stehen
      setSaved(true);
      if (result.data) onStatus(result.data);
    } else {
      setError(result.error.message);
    }
  };

  const remove = async () => {
    setBusy(true);
    setError(null);
    setSaved(false);
    const result = await commands
      .phononGradiumKeyDelete()
      .catch((e) => ({ status: "error" as const, error: asPhononError(e) }));
    setBusy(false);
    if (result.status === "ok") {
      if (result.data) onStatus(result.data);
    } else {
      setError(result.error.message);
    }
  };

  const supported = status?.supported !== false;

  return (
    <div className="space-y-1" data-testid="phonon-key">
      <h4 className="text-xs font-medium text-text/80">
        {t("tts.providers.phononVoices.key.title")}
      </h4>
      <p className="text-xs text-text/70">
        {t("tts.providers.phononVoices.key.hint")}
      </p>
      <p
        className="text-xs text-text/80"
        role="status"
        data-testid="phonon-key-status"
      >
        {!supported
          ? t("tts.providers.phononVoices.key.unsupported")
          : status?.problem
            ? t("tts.providers.phononVoices.key.problem", {
                problem: status.problem,
              })
            : status?.present
              ? t("tts.providers.phononVoices.key.present")
              : t("tts.providers.phononVoices.key.missing")}
      </p>
      {supported && (
        <div className="flex flex-wrap items-center gap-2">
          <Input
            type="password"
            value={value}
            autoComplete="off"
            spellCheck={false}
            className="w-72 max-w-full"
            aria-label={t("tts.providers.phononVoices.key.label")}
            placeholder={t("tts.providers.phononVoices.key.placeholder")}
            data-testid="phonon-key-input"
            onChange={(e) => {
              setValue(e.target.value);
              setSaved(false);
            }}
          />
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={busy || value.trim().length < 8}
            data-testid="phonon-key-save"
            onClick={() => void save()}
          >
            {status?.present
              ? t("tts.providers.phononVoices.key.change")
              : t("tts.providers.phononVoices.key.save")}
          </Button>
          {status?.present && (
            <Button
              type="button"
              variant="danger-ghost"
              size="sm"
              disabled={busy}
              data-testid="phonon-key-delete"
              onClick={() => void remove()}
            >
              {t("tts.providers.phononVoices.key.delete")}
            </Button>
          )}
        </div>
      )}
      {saved && (
        <p
          role="status"
          className="text-xs text-text/80"
          data-testid="phonon-key-saved"
        >
          {t("tts.providers.phononVoices.key.saved")}
        </p>
      )}
      {error && (
        <p
          role="alert"
          className="text-xs text-red-500"
          data-testid="phonon-key-error"
        >
          {error}
        </p>
      )}
    </div>
  );
};
