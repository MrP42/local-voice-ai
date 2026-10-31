import React, { useEffect, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Checkbox } from "@fluentui/react-components";
import {
  commands,
  type HfAccessCheck,
  type HfAccessStatus,
  type TtsProviderEntry,
} from "@/bindings";
import { useTtsModelStore } from "@/stores/ttsModelStore";
import { secretStoreState } from "@/lib/platform/secretStore";
import { Button } from "../../../../ui/Button";
import { formatBytes } from "../../../models/PhononModels";
import { RowChip } from "../../../models/ModelRow";
import type { TtsProvidersState } from "../useTtsProviders";
import {
  buildPlan,
  defaultChoice,
  gatedPending,
  isComplete,
  needsLicenseConsent,
  pendingBytes,
  steps,
  voiceCount,
  type SetupLanguage,
} from "./phononSetupModel";

/** Feste Ziele bei Hugging Face (nur geöffnet, nie mit Zugang aufgerufen). */
const HF_TOKEN_PAGE =
  "https://huggingface.co/settings/tokens/new?tokenType=fineGrained";
const hfModelPage = (info: { source: string | null }) => {
  const repo = info.source?.match(/^hf:\/\/([^@]+)/)?.[1];
  return repo && /^[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/.test(repo)
    ? `https://huggingface.co/${repo}`
    : null;
};

/** Sprachname in der Sprache der Oberfläche („Deutsch“, „Englisch“). */
const languageLabel = (code: string, ui: string) => {
  try {
    const name = new Intl.DisplayNames([ui], { type: "language" }).of(code);
    return name ? name.charAt(0).toUpperCase() + name.slice(1) : code;
  } catch {
    return code;
  }
};

type Phase =
  | { kind: "idle" }
  | { kind: "running"; current: string | null }
  | { kind: "done" }
  | { kind: "error"; message: string };

/**
 * „Einrichten“ für Phonon: ein Klick lädt Programm, Modelle und alle Stimmen,
 * prüft sie (Prüfsummen aus dem Katalog der App) und gibt den Anbieter frei. Der
 * Nutzer wählt nur die Sprachen, bestätigt die Lizenz und – für das gesperrte
 * deutsche Modell – den Hugging-Face-Zugang per Zwischenablage. Keine Pfade.
 */
export const PhononSetup: React.FC<{
  entry: TtsProviderEntry;
  providers: TtsProvidersState;
}> = ({ entry, providers }) => {
  const { t, i18n } = useTranslation();
  const ids = useId();
  const {
    downloads,
    downloadingIds,
    downloadProgress,
    downloadModel,
    cancelDownload,
    loadDownloads,
  } = useTtsModelStore();
  const plan = useMemo(() => buildPlan(downloads), [downloads]);
  const vault = secretStoreState();
  const [access, setAccess] = useState<HfAccessStatus | null>(null);
  const [check, setCheck] = useState<HfAccessCheck | null>(null);
  const [fileOk, setFileOk] = useState(false);
  /** Gesperrte Modelle: mit Tresor immer; ohne (macOS bis lva#134) nur mit Zugang oder Zustimmung zur Klartext-Datei. */
  const gatedPossible = vault.available || (access?.present ?? false) || fileOk;
  const [chosen, setChosen] = useState<Set<string> | null>(null);
  // Ohne Tresor ist die Vorauswahl nur Englisch (Codex 31.10.2026), nie still mehr.
  const choice =
    chosen ??
    defaultChoice(plan, vault.available || (access?.present ?? false));
  const [licenseOk, setLicenseOk] = useState(false);
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [accessBusy, setAccessBusy] = useState(false);
  const [accessError, setAccessError] = useState<string | null>(null);
  /** Eingerichtet: nur eine Zeile; die Teile auf Wunsch. */
  const [showParts, setShowParts] = useState(false);

  useEffect(() => {
    void commands
      .ttsHfAccessStatus()
      .then(setAccess)
      .catch(() => setAccess(null));
  }, []);

  const gated = gatedPending(plan, choice);
  const accessReady = gated.length === 0 || check === "granted";
  const needsActivation =
    (entry.descriptor.opt_in && !entry.enabled) ||
    (entry.license_needs_ack && !entry.license_acked);
  const licenseNeeded =
    needsLicenseConsent(plan, choice) ||
    (entry.license_needs_ack && !entry.license_acked);
  const complete =
    isComplete(plan, choice) && choice.size > 0 && !needsActivation;
  const running = phase.kind === "running";
  const bytes = pendingBytes(plan, choice);
  const canStart =
    !running &&
    choice.size > 0 &&
    !complete &&
    (!licenseNeeded || licenseOk) &&
    accessReady;

  // Liegt ein Zugang vor, still prüfen, ob er für das (erste) gesperrte Modell wirkt –
  // je Modell einmal, auch wenn die Downloads oder die Auswahl erst später kommen.
  const gatedId = gated[0]?.id ?? null;
  const autoCheckedFor = useRef<string | null>(null);
  useEffect(() => {
    if (!access?.present || gatedId === null || check !== null) return;
    if (autoCheckedFor.current === gatedId) return;
    autoCheckedFor.current = gatedId;
    void commands
      .ttsHfAccessVerify(gatedId)
      .then(setCheck)
      .catch(() => setCheck("unreachable"));
  }, [access, gatedId, check]);

  const toggle = (id: string, on: boolean) => {
    const next = new Set(choice);
    if (on) next.add(id);
    else next.delete(id);
    setChosen(next);
  };

  const takeAccess = async () => {
    setAccessBusy(true);
    setAccessError(null);
    try {
      const result = await commands.ttsHfAccessFromClipboard(fileOk);
      if (result.status === "error") {
        setAccessError(result.error);
        return;
      }
      // Die Prüfung folgt hier direkt; die stille Prüfung darf nicht zusätzlich laufen.
      autoCheckedFor.current = gatedId;
      setAccess(result.data);
      const verdict = gated[0]
        ? await commands.ttsHfAccessVerify(gated[0].id)
        : "granted";
      setCheck(verdict);
    } catch (e) {
      setAccessError(String(e));
    } finally {
      setAccessBusy(false);
    }
  };

  const recheckAccess = async () => {
    if (!gated[0]) return;
    setAccessBusy(true);
    setCheck(
      await commands
        .ttsHfAccessVerify(gated[0].id)
        .catch(() => "unreachable" as const),
    );
    setAccessBusy(false);
  };

  const removeAccess = async () => {
    setAccess(await commands.ttsHfAccessDelete().catch(() => null));
    setCheck(null);
  };

  const run = async () => {
    if (running) return;
    const todo = steps(plan, choice);
    // Sofort sperren: ein Doppelklick darf keinen zweiten Lauf starten.
    setPhase({ kind: "running", current: null });
    try {
      if (entry.descriptor.opt_in && !entry.enabled) {
        const en = await commands.ttsProviderSetEnabled(
          entry.descriptor.id,
          true,
        );
        if (en.status === "error") throw new Error(en.error.message);
      }
      for (const id of todo) {
        setPhase({ kind: "running", current: id });
        const ok = await downloadModel(id);
        await loadDownloads();
        if (!ok) {
          setPhase({
            kind: "error",
            message: t("tts.providers.setup.failed", { name: nameOf(id) }),
          });
          return;
        }
        // Abgebrochen (Download meldet Erfolg, ist aber nicht installiert): still anhalten.
        const row = useTtsModelStore
          .getState()
          .downloads.find((d) => d.id === id);
        if (row && !row.is_downloaded) {
          setPhase({ kind: "idle" });
          return;
        }
      }
      // Erst jetzt, mit dem eingerichteten Modell, gilt der Fingerabdruck der Lizenz.
      const list = await providers.reload();
      const fresh = list?.find((e) => e.descriptor.id === entry.descriptor.id);
      if (
        fresh &&
        fresh.license_needs_ack &&
        !fresh.license_acked &&
        licenseOk
      ) {
        const ack = await commands.ttsProviderAcknowledgeLicense(
          fresh.descriptor.id,
          fresh.fingerprint,
        );
        if (ack.status === "error") throw new Error(ack.error.message);
      }
      await providers.recheck(entry.descriptor.id);
      providers.changed();
      setPhase({ kind: "done" });
    } catch (e) {
      setPhase({
        kind: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  };

  const nameOf = (id: string) => {
    if (plan.runtime?.id === id) return t("tts.providers.setup.program");
    const l = plan.languages.find((x) => x.id === id);
    return l ? languageName(l) : id;
  };
  const languageName = (l: SetupLanguage) =>
    t("tts.providers.setup.languageModel", {
      language: languageLabel(l.language, i18n.language),
      name: l.info.name.replace(/\s*\(.*\)$/, ""),
    });

  const status = (() => {
    if (running && phase.current)
      return t("tts.providers.setup.statusRunning", {
        name: nameOf(phase.current),
      });
    if (phase.kind === "done" || complete)
      return t("tts.providers.setup.statusDone", {
        count: voiceCount(plan, choice),
      });
    if (phase.kind === "error") return phase.message;
    return "";
  })();

  if (!plan.runtime && plan.languages.length === 0) return null;

  const progressRow = (id: string) => {
    const p = downloadProgress[id];
    if (!(id in downloadingIds) || !p) return null;
    const pct = Math.max(0, Math.min(100, Math.round(p.percentage)));
    return (
      <div className="mt-1 space-y-1">
        <div
          role="progressbar"
          aria-label={t("tts.providers.setup.progressLabel", {
            name: nameOf(id),
          })}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={pct}
          className="h-1.5 overflow-hidden rounded bg-mid-gray/20"
        >
          <div
            className="h-full bg-logo-primary"
            style={{ width: `${pct}%` }}
          />
        </div>
        <div className="flex items-center gap-2 text-xs text-text/70">
          <span>
            {t("tts.providers.setup.progressBytes", {
              done: formatBytes(p.downloaded),
              total: formatBytes(p.total),
            })}
          </span>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="min-h-[44px] md:min-h-6"
            data-testid={`phonon-setup-cancel-${id}`}
            onClick={() => void cancelDownload(id)}
          >
            {t("tts.providers.setup.cancel")}
          </Button>
        </div>
      </div>
    );
  };

  return (
    <section
      aria-labelledby={`${ids}-title`}
      className="rounded-lg border border-mid-gray/25"
      data-testid="phonon-setup"
      data-complete={complete ? "true" : "false"}
    >
      <div
        className={`flex items-start gap-3 rounded-t-lg px-3 py-2.5 ${
          complete ? "bg-green-500/10" : "bg-logo-primary/10"
        }`}
      >
        <div className="min-w-0">
          <h3 id={`${ids}-title`} className="text-sm font-semibold text-text">
            {complete
              ? t("tts.providers.setup.titleDone")
              : t("tts.providers.setup.title")}
          </h3>
          <p className="text-xs text-text/70">
            {complete
              ? t("tts.providers.setup.summaryDone", {
                  languages: plan.languages
                    .filter((l) => l.complete)
                    .map((l) => languageLabel(l.language, i18n.language))
                    .join(", "),
                  count: voiceCount(plan, choice),
                })
              : t("tts.providers.setup.intro")}
          </p>
        </div>
        {complete && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="ms-auto min-h-[44px] shrink-0 md:min-h-8"
            aria-expanded={showParts}
            data-testid="phonon-setup-parts"
            onClick={() => setShowParts((v) => !v)}
          >
            {showParts
              ? t("tts.providers.setup.partsHide")
              : t("tts.providers.setup.partsShow")}
          </Button>
        )}
      </div>

      {(!complete || showParts) && (
        <ul className="divide-y divide-mid-gray/15">
          {plan.runtime && (
            <li className="grid grid-cols-[minmax(0,1fr)_auto] gap-2 px-3 py-2">
              <div className="min-w-0">
                <div className="text-sm font-medium">
                  {plan.runtime.is_downloaded ? "✓ " : ""}
                  {t("tts.providers.setup.program")}
                </div>
                <div className="text-xs text-text/60">
                  {plan.runtime.license}
                </div>
                {progressRow(plan.runtime.id)}
              </div>
              <span className="text-xs text-text/60">
                {formatBytes(plan.runtime.size_bytes)}
              </span>
            </li>
          )}
          {plan.languages.map((l) => {
            const blockedHere =
              l.gated && !gatedPossible && !l.info.is_downloaded;
            const cbId = `${ids}-lang-${l.id}`;
            return (
              <li
                key={l.id}
                className="grid grid-cols-[minmax(0,1fr)_auto] gap-2 px-3 py-2"
                data-testid={`phonon-setup-lang-${l.id}`}
              >
                <div className="min-w-0">
                  <Checkbox
                    id={cbId}
                    checked={choice.has(l.id)}
                    disabled={running || l.info.is_downloaded || blockedHere}
                    data-testid={`phonon-setup-choose-${l.id}`}
                    onChange={(_e, d) => toggle(l.id, d.checked === true)}
                    label={
                      <span className="text-sm font-medium">
                        {l.complete ? "✓ " : ""}
                        {languageName(l)}
                      </span>
                    }
                  />
                  <div className="ps-8 text-xs text-text/60">
                    {t("tts.providers.setup.voices", { count: l.voices })} ·{" "}
                    {l.info.license}
                    {l.nonCommercial && (
                      <span className="ms-1">
                        <RowChip tone="warning">
                          {t("tts.providers.license.nonCommercial")}
                        </RowChip>
                      </span>
                    )}
                    {l.gated &&
                      !vault.available &&
                      !access?.present &&
                      !l.info.is_downloaded && (
                        <span
                          className="block text-amber-800 dark:text-amber-300"
                          data-testid={`phonon-setup-mac-hint-${l.id}`}
                        >
                          {t("tts.providers.setup.gatedNeedsVault")}
                        </span>
                      )}
                  </div>
                  {l.gated &&
                    !vault.available &&
                    !access?.present &&
                    !l.info.is_downloaded && (
                      <div className="ps-6">
                        <Checkbox
                          checked={fileOk}
                          disabled={running}
                          data-testid="phonon-setup-access-file"
                          onChange={(_e, d) => {
                            const on = d.checked === true;
                            setFileOk(on);
                            if (!on) toggle(l.id, false);
                          }}
                          label={
                            <span className="text-xs">
                              {t("tts.providers.setup.access.fileConsent")}
                            </span>
                          }
                        />
                      </div>
                    )}
                  {progressRow(l.id)}
                </div>
                <span className="text-xs text-text/60">
                  {formatBytes(l.info.size_bytes)}
                </span>
              </li>
            );
          })}
        </ul>
      )}

      {!complete && licenseNeeded && (
        <div className="border-t border-mid-gray/15 bg-mid-gray/5 px-3 py-2">
          <Checkbox
            checked={licenseOk}
            disabled={running}
            data-testid="phonon-setup-license"
            onChange={(_e, d) => setLicenseOk(d.checked === true)}
            label={
              <span className="text-xs">
                {t("tts.providers.setup.licenseConsent")}
              </span>
            }
          />
          {plan.languages
            .filter((l) => choice.has(l.id) && l.nonCommercial)
            .map((l) =>
              l.info.license_url ? (
                <button
                  key={l.id}
                  type="button"
                  className="ms-8 min-h-6 text-xs underline"
                  onClick={() => void openUrl(l.info.license_url as string)}
                >
                  {t("tts.providers.setup.licenseOpen", {
                    name: languageName(l),
                  })}
                </button>
              ) : null,
            )}
        </div>
      )}

      {!complete && gated.length > 0 && (
        <div
          className="space-y-2 border-t border-mid-gray/15 bg-mid-gray/5 px-3 py-2"
          data-testid="phonon-setup-access"
          data-check={check ?? "none"}
        >
          <div className="text-sm font-medium">
            {t("tts.providers.setup.access.title")}
          </div>
          {check === "granted" ? (
            <p className="text-xs text-green-700 dark:text-green-400">
              {"✓ "}
              {t("tts.providers.setup.access.granted")}{" "}
              {access?.source === "app" ? (
                <button
                  type="button"
                  className="min-h-6 underline"
                  data-testid="phonon-setup-access-remove"
                  onClick={() => void removeAccess()}
                >
                  {t("tts.providers.setup.access.remove")}
                </button>
              ) : (
                <span
                  className="text-text/60"
                  data-testid="phonon-setup-access-system"
                >
                  {t("tts.providers.setup.access.systemOwned")}
                </span>
              )}
            </p>
          ) : (
            <>
              <p className="text-xs text-text/70">
                {t("tts.providers.setup.access.steps")}
              </p>
              <p className="text-xs text-text/60">
                {t("tts.providers.setup.access.clipboardRisk")}
              </p>
              <div className="flex flex-wrap gap-2">
                {hfModelPage(gated[0].info) && (
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    className="min-h-[44px] md:min-h-8"
                    onClick={() =>
                      void openUrl(hfModelPage(gated[0].info) as string)
                    }
                  >
                    {t("tts.providers.setup.access.openTerms")}
                  </Button>
                )}
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  className="min-h-[44px] md:min-h-8"
                  onClick={() => void openUrl(HF_TOKEN_PAGE)}
                >
                  {t("tts.providers.setup.access.openToken")}
                </Button>
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  className="min-h-[44px] md:min-h-8"
                  disabled={accessBusy || (!vault.available && !fileOk)}
                  data-testid="phonon-setup-access-paste"
                  onClick={() => void takeAccess()}
                >
                  {t("tts.providers.setup.access.paste")}
                </Button>
                {access?.present && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="min-h-[44px] md:min-h-8"
                    disabled={accessBusy}
                    onClick={() => void recheckAccess()}
                  >
                    {t("tts.providers.setup.access.recheck")}
                  </Button>
                )}
              </div>
              {(check === "denied" ||
                check === "unreachable" ||
                accessError) && (
                <p
                  role="alert"
                  className="text-xs text-red-600 dark:text-red-400"
                >
                  {accessError ?? t(`tts.providers.setup.access.${check}`)}
                </p>
              )}
            </>
          )}
        </div>
      )}

      <div
        className={`flex flex-wrap items-center gap-3 px-3 ${
          complete && phase.kind !== "error"
            ? "sr-only"
            : "border-t border-mid-gray/15 py-2.5"
        }`}
      >
        {!complete && (
          <Button
            type="button"
            variant="primary"
            size="md"
            className="min-h-[44px] md:min-h-8"
            disabled={!canStart}
            // Kein Überblenden aus dem Sperrzustand: jeder Zwischenframe hätte < 4,5:1.
            style={{ transition: "none" }}
            data-testid="phonon-setup-start"
            onClick={() => void run()}
          >
            {running
              ? t("tts.providers.setup.running")
              : t("tts.providers.setup.start", { size: formatBytes(bytes) })}
          </Button>
        )}
        {!complete && !canStart && !running && (
          <span className="text-xs text-text/60" data-testid="phonon-setup-why">
            {choice.size === 0
              ? t("tts.providers.setup.whyNothing")
              : licenseNeeded && !licenseOk
                ? t("tts.providers.setup.whyLicense")
                : !accessReady
                  ? t("tts.providers.setup.whyAccess")
                  : ""}
          </span>
        )}
        {phase.kind === "error" && (
          <Button
            type="button"
            variant="secondary"
            size="sm"
            className="min-h-[44px] md:min-h-8"
            data-testid="phonon-setup-retry"
            onClick={() => void run()}
          >
            {t("tts.providers.setup.retry")}
          </Button>
        )}
        <span
          role="status"
          aria-live="polite"
          className={`text-xs ${
            phase.kind === "error"
              ? "text-red-600 dark:text-red-400"
              : "text-text/70"
          }`}
          data-testid="phonon-setup-status"
        >
          {status}
        </span>
      </div>
    </section>
  );
};
