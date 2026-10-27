import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { Checkbox } from "@fluentui/react-components";
import { FolderOpen, Mic, Square } from "lucide-react";
import {
  commands,
  type KeyStatus,
  type OwnVoice,
  type VoiceInfo,
} from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import {
  AUDIO_EXTENSIONS,
  CLONE_LANGUAGES,
  MAX_RECORDING_S,
  NAME_MAX,
  PHASE_EVENT,
  asPhononError,
  buildSource,
  canRunClone,
  fileBaseName,
  nameFromFile,
  parseStart,
  type ClonePhase,
  type SourceKind,
} from "./phononVoicesModel";

interface Props {
  open: boolean;
  /** „Fish-Stimme nach Phonon übernehmen“: diese Fish-Stimme ist vorgewählt. */
  initialFishId: string | null;
  fishVoices: VoiceInfo[];
  /** Sprachen, die Gradium annimmt (Reihenfolge der Anzeige bestimmt die Oberfläche). */
  languages: string[];
  keyStatus: KeyStatus | null;
  onClose: () => void;
  onDone: (voice: OwnVoice) => void;
}

type Rec = { state: "idle" | "recording" | "ready"; seconds: number };

/**
 * „Stimme erstellen“: aus einer Audiodatei, einer Mikrofonaufnahme oder der
 * Referenzaufnahme einer Fish-Stimme. Das ist der EINE Weg, auf dem eine Aufnahme
 * den Rechner verlässt (Gradium, api.gradium.ai): der Hinweis steht immer da, das
 * Häkchen „Die sprechende Person hat eingewilligt“ gilt je Stimme (es wird bei
 * jeder neuen Quelle zurückgesetzt), und ohne beides geht kein Netzzugriff los —
 * das Backend prüft die Einwilligung noch einmal.
 */
export const PhononCloneDialog: React.FC<Props> = ({
  open,
  initialFishId,
  fishVoices,
  languages,
  keyStatus,
  onClose,
  onDone,
}) => {
  const { t, i18n } = useTranslation();
  const [kind, setKind] = useState<SourceKind>("file");
  const [path, setPath] = useState("");
  const [fishId, setFishId] = useState("");
  const [start, setStart] = useState("0");
  const [language, setLanguage] = useState("de");
  const [name, setName] = useState("");
  const [consent, setConsent] = useState(false);
  const [running, setRunning] = useState(false);
  const [phase, setPhase] = useState<ClonePhase | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [askOverwrite, setAskOverwrite] = useState(false);
  const [done, setDone] = useState<OwnVoice | null>(null);
  const [rec, setRec] = useState<Rec>({ state: "idle", seconds: 0 });
  const [recToken, setRecToken] = useState(0);
  const nameTouched = useRef(false);
  const timer = useRef<number | null>(null);
  const startedAt = useRef(0);

  const fishName = (id: string) => {
    const v = fishVoices.find((x) => x.id === id);
    return (v?.meta.display_name || id).slice(0, NAME_MAX);
  };

  const stopTimer = () => {
    if (timer.current !== null) {
      window.clearInterval(timer.current);
      timer.current = null;
    }
  };

  // Beim Öffnen frisch: nichts übernommen, keine Einwilligung.
  useEffect(() => {
    if (!open) return;
    stopTimer();
    setKind(initialFishId ? "fish" : "file");
    setPath("");
    setFishId(initialFishId ?? "");
    setStart("0");
    setLanguage("de");
    setName(initialFishId ? fishName(initialFishId) : "");
    nameTouched.current = false;
    setConsent(false);
    setRunning(false);
    setPhase(null);
    setError(null);
    setAskOverwrite(false);
    setDone(null);
    setRec({ state: "idle", seconds: 0 });
    // `fishName` hängt an fishVoices; nur beim Öffnen vorbelegen.
  }, [open, initialFishId]);

  // Einwilligung gilt für GENAU die gewählte Quelle: eine andere Quelle, eine neue
  // Aufnahme oder eine andere Fish-Stimme setzen sie zurück.
  useEffect(() => {
    setConsent(false);
    setAskOverwrite(false);
  }, [kind, path, fishId, recToken]);

  useEffect(() => {
    if (!open) return;
    const un = listen<ClonePhase>(PHASE_EVENT, (e) => setPhase(e.payload));
    return () => {
      void un.then((f) => f());
    };
  }, [open]);

  useEffect(() => () => stopTimer(), []);

  const languageName = (code: string) => {
    try {
      return (
        new Intl.DisplayNames([i18n.language], { type: "language" }).of(code) ??
        code
      );
    } catch {
      return code;
    }
  };

  const languageOptions = useMemo(() => {
    const offered = languages.length > 0 ? languages : [...CLONE_LANGUAGES];
    const ordered = [
      ...CLONE_LANGUAGES.filter((l) => offered.includes(l)),
      ...offered.filter(
        (l) => !(CLONE_LANGUAGES as readonly string[]).includes(l),
      ),
    ];
    return ordered.map((code) => ({ value: code, label: languageName(code) }));
  }, [languages, i18n.language]);

  const fishOptions = useMemo(
    () =>
      fishVoices.map((v) => ({
        value: v.id,
        label: v.meta.display_name || v.id,
      })),
    [fishVoices],
  );

  const keyPresent = keyStatus?.present === true;
  const recorded = rec.state === "ready";
  const ready = canRunClone({
    kind,
    path,
    fishId,
    recorded,
    name,
    start,
    consent,
    keyPresent,
    busy: running || rec.state === "recording",
  });

  const discardRecording = () => {
    stopTimer();
    if (rec.state !== "idle") void commands.phononVoiceRecordDiscard();
  };

  const close = () => {
    if (running) return;
    discardRecording();
    onClose();
  };

  const pickFile = async () => {
    try {
      const picked = await openFile({
        multiple: false,
        filters: [
          {
            name: t("tts.providers.phononVoices.clone.audioFilter"),
            extensions: AUDIO_EXTENSIONS,
          },
        ],
      });
      if (typeof picked === "string") {
        setPath(picked);
        if (!nameTouched.current) setName(nameFromFile(picked));
        setError(null);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const stopRecording = async () => {
    stopTimer();
    const result = await commands
      .phononVoiceRecordStop()
      .catch((e) => ({ status: "error" as const, error: asPhononError(e) }));
    if (result.status === "error") {
      setRec({ state: "idle", seconds: 0 });
      setError(result.error.message);
      return;
    }
    setRec({ state: "ready", seconds: result.data.seconds });
    setRecToken((n) => n + 1);
  };
  const stopRef = useRef(stopRecording);
  stopRef.current = stopRecording;

  const startRecording = async () => {
    setError(null);
    const result = await commands
      .phononVoiceRecordStart()
      .catch((e) => ({ status: "error" as const, error: asPhononError(e) }));
    if (result.status === "error") {
      setError(result.error.message);
      return;
    }
    startedAt.current = Date.now();
    setRec({ state: "recording", seconds: 0 });
    stopTimer();
    timer.current = window.setInterval(() => {
      const s = Math.floor((Date.now() - startedAt.current) / 1000);
      setRec((prev) =>
        prev.state === "recording" ? { state: "recording", seconds: s } : prev,
      );
      if (s >= MAX_RECORDING_S) void stopRef.current();
    }, 250);
  };

  const run = async (overwrite: boolean) => {
    const source = buildSource({ kind, path, fishId, recorded });
    const startS = parseStart(start);
    if (source === null || startS === null) return;
    setRunning(true);
    setPhase("uploading");
    setError(null);
    const result = await commands
      .phononVoiceClone({
        source,
        name: name.trim(),
        language,
        start_s: kind === "recording" ? 0 : startS,
        consent,
        overwrite,
      })
      .catch((e) => ({ status: "error" as const, error: asPhononError(e) }));
    setRunning(false);
    setPhase(null);
    if (result.status === "ok") {
      if (kind === "recording") setRec({ state: "idle", seconds: 0 });
      setDone(result.data);
      onDone(result.data);
      return;
    }
    if (result.error.kind === "exists") {
      setAskOverwrite(true);
      return;
    }
    if (result.error.kind === "cancelled") return;
    setAskOverwrite(false);
    setError(result.error.message);
  };

  const sources: { kind: SourceKind; label: string }[] = [
    { kind: "file", label: t("tts.providers.phononVoices.clone.source.file") },
    {
      kind: "recording",
      label: t("tts.providers.phononVoices.clone.source.mic"),
    },
    { kind: "fish", label: t("tts.providers.phononVoices.clone.source.fish") },
  ];

  const footer = done ? (
    <Button type="button" variant="primary" onClick={onClose}>
      {t("common.close")}
    </Button>
  ) : running ? (
    <Button
      type="button"
      variant="secondary"
      data-testid="phonon-clone-cancel"
      onClick={() => void commands.phononVoiceCloneCancel()}
    >
      {t("tts.providers.phononVoices.clone.cancel")}
    </Button>
  ) : (
    <>
      <Button
        type="button"
        variant="primary"
        disabled={!ready}
        data-testid="phonon-clone-run"
        onClick={() => void run(askOverwrite)}
      >
        {askOverwrite
          ? t("tts.providers.phononVoices.clone.replace")
          : t("tts.providers.phononVoices.clone.run")}
      </Button>
      <Button type="button" variant="secondary" onClick={close}>
        {t("common.cancel")}
      </Button>
    </>
  );

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
      title={t("tts.providers.phononVoices.clone.title")}
      description={t("tts.providers.phononVoices.clone.description")}
      closeLabel={t("common.close")}
      dismissible={!running}
      className="max-w-xl"
      footer={footer}
    >
      <div className="space-y-4 text-sm" data-testid="phonon-clone-dialog">
        {done ? (
          <p role="status" data-testid="phonon-clone-done">
            {t(
              done.in_models.length > 0
                ? "tts.providers.phononVoices.clone.done"
                : "tts.providers.phononVoices.clone.doneNoModel",
              { name: done.name },
            )}
          </p>
        ) : (
          <>
            <fieldset className="space-y-2" disabled={running}>
              <legend className="text-xs font-medium text-text/80">
                {t("tts.providers.phononVoices.clone.source.label")}
              </legend>
              <div className="flex flex-wrap gap-4">
                {sources.map((s) => (
                  <label
                    key={s.kind}
                    className="flex cursor-pointer items-center gap-1.5"
                  >
                    <input
                      type="radio"
                      name="phonon-clone-source"
                      checked={kind === s.kind}
                      onChange={() => setKind(s.kind)}
                      data-testid={`phonon-clone-source-${s.kind}`}
                    />
                    {s.label}
                  </label>
                ))}
              </div>

              {kind === "file" && (
                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    data-testid="phonon-clone-file-choose"
                    onClick={() => void pickFile()}
                  >
                    <FolderOpen width={14} height={14} />
                    {t("tts.providers.phononVoices.clone.chooseFile")}
                  </Button>
                  <span
                    className="min-w-0 truncate text-xs text-text/70"
                    data-testid="phonon-clone-file"
                  >
                    {path
                      ? fileBaseName(path)
                      : t("tts.providers.phononVoices.clone.noFile")}
                  </span>
                </div>
              )}

              {kind === "fish" && (
                <div className="space-y-1">
                  {fishVoices.length === 0 ? (
                    <p className="text-xs text-text/70">
                      {t("tts.providers.phononVoices.fish.empty")}
                    </p>
                  ) : (
                    <Select
                      value={fishId || null}
                      options={fishOptions}
                      isClearable={false}
                      menuPortal
                      placeholder={t(
                        "tts.providers.phononVoices.clone.fishPlaceholder",
                      )}
                      ariaLabel={t(
                        "tts.providers.phononVoices.clone.source.fish",
                      )}
                      onChange={(v) => {
                        setFishId(v ?? "");
                        if (v && !nameTouched.current) setName(fishName(v));
                      }}
                    />
                  )}
                </div>
              )}

              {kind === "recording" && (
                <div className="space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    {rec.state === "recording" ? (
                      <Button
                        type="button"
                        variant="secondary"
                        size="sm"
                        data-testid="phonon-clone-record-stop"
                        onClick={() => void stopRecording()}
                      >
                        <Square width={14} height={14} />
                        {t("tts.providers.phononVoices.clone.recordStop")}
                      </Button>
                    ) : (
                      <Button
                        type="button"
                        variant="secondary"
                        size="sm"
                        data-testid="phonon-clone-record-start"
                        onClick={() => void startRecording()}
                      >
                        <Mic width={14} height={14} />
                        {rec.state === "ready"
                          ? t("tts.providers.phononVoices.clone.recordAgain")
                          : t("tts.providers.phononVoices.clone.recordStart")}
                      </Button>
                    )}
                    <span
                      className="text-xs tabular-nums text-text/70"
                      role="status"
                      data-testid="phonon-clone-record-status"
                    >
                      {rec.state === "recording" &&
                        t("tts.providers.phononVoices.clone.recording", {
                          seconds: rec.seconds,
                        })}
                      {rec.state === "ready" &&
                        t("tts.providers.phononVoices.clone.recorded", {
                          seconds: rec.seconds.toFixed(1),
                        })}
                    </span>
                  </div>
                  <p className="text-xs text-text/70">
                    {t("tts.providers.phononVoices.clone.recordHint")}
                  </p>
                </div>
              )}
            </fieldset>

            {kind !== "recording" && (
              <div className="space-y-1">
                <label
                  htmlFor="phonon-clone-start"
                  className="block text-xs font-medium text-text/80"
                >
                  {t("tts.providers.phononVoices.clone.start")}
                </label>
                <Input
                  id="phonon-clone-start"
                  type="text"
                  inputMode="decimal"
                  value={start}
                  disabled={running}
                  className="w-28"
                  data-testid="phonon-clone-start"
                  aria-invalid={parseStart(start) === null}
                  onChange={(e) => setStart(e.target.value)}
                />
                <p className="text-xs text-text/70">
                  {t("tts.providers.phononVoices.clone.startHint")}
                </p>
              </div>
            )}

            <div className="grid gap-3 sm:grid-cols-2">
              <div className="min-w-0 space-y-1">
                <span className="block text-xs font-medium text-text/80">
                  {t("tts.providers.phononVoices.clone.language")}
                </span>
                <Select
                  value={language}
                  options={languageOptions}
                  isClearable={false}
                  menuPortal
                  disabled={running}
                  ariaLabel={t("tts.providers.phononVoices.clone.language")}
                  onChange={(v) => setLanguage(v ?? "de")}
                />
              </div>
              <div className="min-w-0 space-y-1">
                <label
                  htmlFor="phonon-clone-name"
                  className="block text-xs font-medium text-text/80"
                >
                  {t("tts.providers.phononVoices.clone.name")}
                </label>
                <Input
                  id="phonon-clone-name"
                  type="text"
                  value={name}
                  maxLength={NAME_MAX}
                  autoComplete="off"
                  disabled={running}
                  className="w-full"
                  data-testid="phonon-clone-name"
                  onChange={(e) => {
                    nameTouched.current = true;
                    setName(e.target.value);
                    setAskOverwrite(false);
                  }}
                />
              </div>
            </div>

            {/* Pflicht-Hinweis: steht immer da, vor dem Senden. */}
            <div
              className="space-y-2 rounded-md border border-yellow-500/40 bg-yellow-500/10 p-3"
              data-testid="phonon-clone-notice"
            >
              <p className="font-medium">
                {t("tts.providers.phononVoices.clone.notice")}
              </p>
              <p className="text-xs text-text/80">
                {t("tts.providers.phononVoices.clone.noticeMore")}
              </p>
              <Checkbox
                checked={consent}
                disabled={running}
                onChange={(_e, data) => setConsent(data.checked === true)}
                label={t("tts.providers.phononVoices.clone.consent")}
                data-testid="phonon-clone-consent"
              />
            </div>

            {!keyPresent && (
              <p
                role="alert"
                className="text-xs text-yellow-500"
                data-testid="phonon-clone-nokey"
              >
                {t("tts.providers.phononVoices.clone.noKey")}
              </p>
            )}
            {running && (
              <p
                role="status"
                className="text-xs text-text/80"
                data-testid="phonon-clone-phase"
              >
                {t(
                  `tts.providers.phononVoices.clone.phase.${phase ?? "uploading"}`,
                )}
              </p>
            )}
            {askOverwrite && (
              <p
                role="alert"
                className="text-xs text-yellow-500"
                data-testid="phonon-clone-overwrite"
              >
                {t("tts.providers.phononVoices.clone.overwrite", {
                  name: name.trim(),
                })}
              </p>
            )}
            {error && (
              <p
                role="alert"
                className="text-xs text-red-500"
                data-testid="phonon-clone-error"
              >
                {error}
              </p>
            )}
          </>
        )}
      </div>
    </Dialog>
  );
};
