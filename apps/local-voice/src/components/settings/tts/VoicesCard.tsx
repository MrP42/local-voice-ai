import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { commands, type VoiceInfo, type VoiceSample } from "@/bindings";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Pencil, Play, Trash2, Upload, Wand2 } from "lucide-react";
import { AudioPlayer } from "../../ui/AudioPlayer";
import { useSettings } from "../../../hooks/useSettings";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { Input } from "../../ui/Input";
import { Textarea } from "../../ui/Textarea";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import Badge from "../../ui/Badge";
import { VoiceBuilder } from "./builder";
import { VoiceArchiveImport, VoiceEditor } from "./voices";

type Mode =
  | { kind: "idle" }
  | { kind: "recording" }
  | { kind: "review"; source: "recording" | { wavPath: string } };

/// Kennung der Standardstimme gegenueber dem Backend: kein Referenzname,
/// also die leere Zeichenkette. Ein eigener Name waere ein Stimmenname, den
/// jemand fuer eine echte Stimme vergeben koennte.
const SEED_VOICE = "";

export const VoicesCard = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();
  // Die Liste traegt seit Etappe 5 die Metadaten mit: das Bearbeiten-Panel
  // braucht den Anzeigenamen, und die Zeile soll ihn zeigen statt nur der id.
  const [voices, setVoices] = useState<VoiceInfo[]>([]);
  const [mode, setMode] = useState<Mode>({ kind: "idle" });
  const [name, setName] = useState("");
  const [transcript, setTranscript] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [recordSeconds, setRecordSeconds] = useState(0);
  // Laeuft gerade eine Transkription der gewaehlten Datei? Sie dauert je
  // nach Modell und Laenge einige Sekunden — ohne sichtbaren Zustand
  // sieht das Textfeld einfach nur leer aus, und niemand weiss, ob da
  // noch etwas kommt.
  const [transcribing, setTranscribing] = useState(false);
  const recordTimer = useRef<number | null>(null);
  // Which voice the user opened a preview for, and what it is. Loaded on
  // demand rather than for every voice up front: a preview is a file read, and
  // most of the time you only want to hear one of them.
  const [sample, setSample] = useState<{
    id: string;
    data: VoiceSample | null;
    error?: string;
  } | null>(null);
  const [previewing, setPreviewing] = useState<string | null>(null);
  // Deleting a voice throws away a recording that cannot be reproduced — the
  // same person has to sit down and speak again. That deserves a question,
  // especially since the button sits right next to "Activate".
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  // Der Stimmen-Baukasten liegt in dieser Karte statt in einem eigenen Reiter
  // (AGENTS.md: kein neuer Menuepunkt) und ist zugeklappt, bis jemand ihn
  // aufmacht — die Karte ist ohnehin lang genug.
  const [builderOpen, setBuilderOpen] = useState(false);
  // Welche Stimme gerade bearbeitet wird — hoechstens eine, sonst wird die
  // Karte unlesbar lang. Der Stift schaltet das Panel auf und wieder zu.
  const [editTarget, setEditTarget] = useState<string | null>(null);

  const activeVoice = getSetting("tts_voice") ?? null;

  const refreshVoices = useCallback(async () => {
    setVoices(await commands.ttsListVoiceInfos());
    // Das Dropdown an der Transportleiste haelt seine eigene Liste — dieses
    // Ereignis haelt beide zusammen, ohne dass sie sich kennen muessen.
    window.dispatchEvent(new CustomEvent("lv-voices-changed"));
  }, []);

  useEffect(() => {
    void refreshVoices();
  }, [refreshVoices]);

  useEffect(() => {
    if (mode.kind === "recording") {
      setRecordSeconds(0);
      recordTimer.current = window.setInterval(
        () => setRecordSeconds((s) => s + 1),
        1000,
      );
    } else if (recordTimer.current !== null) {
      window.clearInterval(recordTimer.current);
      recordTimer.current = null;
    }
    return () => {
      if (recordTimer.current !== null) {
        window.clearInterval(recordTimer.current);
        recordTimer.current = null;
      }
    };
  }, [mode.kind]);

  const togglePreview = async (id: string) => {
    if (sample?.id === id) {
      setSample(null);
      return;
    }
    setSample(null);
    setPreviewing(id);
    // Generated once per voice and cached; the first click for a voice has to
    // wait for Fish Speech (and may have to start it), later ones are instant.
    const result = await commands.ttsVoiceDemo(id);
    setPreviewing(null);
    setSample(
      result.status === "ok"
        ? { id, data: result.data }
        : { id, data: null, error: result.error },
    );
  };

  const startRecording = async () => {
    setError(null);
    const result = await commands.ttsRecordReferenceStart();
    if (result.status === "error") {
      setError(result.error);
      return;
    }
    setMode({ kind: "recording" });
  };

  const stopRecording = async () => {
    setBusy(true);
    const result = await commands.ttsRecordReferenceStop();
    setBusy(false);
    if (result.status === "error") {
      setError(result.error);
      setMode({ kind: "idle" });
      return;
    }
    setTranscript(result.data);
    setMode({ kind: "review", source: "recording" });
  };

  const pickFile = async () => {
    setError(null);
    const picked = await open({
      multiple: false,
      filters: [
        {
          name: "Audio/Video",
          extensions: [
            "wav",
            "mp3",
            "m4a",
            "aac",
            "flac",
            "ogg",
            "opus",
            "wma",
            "mp4",
            "mov",
            "mkv",
            "webm",
            "avi",
          ],
        },
      ],
    });
    if (typeof picked !== "string") return;
    setTranscript("");
    setMode({ kind: "review", source: { wavPath: picked } });
    // Transkribieren, sobald die Datei feststeht — nicht erst beim
    // Speichern. Sonst steht der Text erst da, wenn die Stimme schon
    // angelegt ist, und niemand kann ihn vorher noch berichtigen.
    if (!(getSetting("tts_reference_auto_transcribe") ?? true)) return;
    setTranscribing(true);
    const spoken = await commands.ttsTranscribeReference(picked);
    setTranscribing(false);
    if (spoken.status === "ok") {
      setTranscript(spoken.data);
    } else {
      // Kein Abbruch: die Aufnahme taugt weiter, nur der Text fehlt.
      setError(spoken.error);
    }
  };

  const save = async () => {
    if (mode.kind !== "review") return;
    setBusy(true);
    setError(null);
    const result =
      mode.source === "recording"
        ? await commands.ttsSaveVoice(name, transcript)
        : await commands.ttsImportVoice(
            name,
            mode.source.wavPath,
            transcript.trim().length > 0 ? transcript : null,
          );
    setBusy(false);
    if (result.status === "error") {
      setError(result.error);
      return;
    }
    const id = typeof result.data === "string" ? result.data : result.data.id;
    setMode({ kind: "idle" });
    setName("");
    setTranscript("");
    await refreshVoices();
    // Neue Stimme direkt aktiv schalten — das ist praktisch immer die Absicht.
    await updateSetting("tts_voice", id);
  };

  const discard = () => {
    setMode({ kind: "idle" });
    setName("");
    setTranscript("");
    setError(null);
  };

  const remove = async (id: string) => {
    setDeleteTarget(null);
    setError(null);
    const result = await commands.ttsDeleteVoice(id);
    if (result.status === "error") {
      setError(result.error);
      return;
    }
    if (activeVoice === id) {
      await updateSetting("tts_voice", null);
    }
    await refreshVoices();
  };

  // Nach dem Baukasten dasselbe wie nach einer Aufnahme: Liste neu laden
  // (das loest `lv-voices-changed` aus, damit Transportleiste und
  // Sprecher-Chips nachziehen) und die frische Stimme aktiv schalten.
  const builderSaved = async (id: string) => {
    setBuilderOpen(false);
    await refreshVoices();
    window.dispatchEvent(new CustomEvent("lv-voices-changed"));
    await updateSetting("tts_voice", id);
  };

  // Nach jeder Aenderung im Bearbeiten-Panel: Liste neu laden. Beim Umzug der
  // voice_id zieht die aktive Stimme nach — sonst zeigt die Einstellung auf
  // einen Ordner, den es nicht mehr gibt.
  const voiceEdited = async (newId?: string) => {
    if (newId !== undefined && newId !== editTarget) {
      if (activeVoice === editTarget) {
        await updateSetting("tts_voice", newId);
      }
      setEditTarget(newId);
    }
    await refreshVoices();
  };

  // Eine eingespielte Stimme ist wie eine frisch aufgenommene: Liste neu
  // laden und die neue Stimme aktiv schalten.
  const archiveImported = async (id: string) => {
    await refreshVoices();
    await updateSetting("tts_voice", id);
  };

  return (
    <SettingsGroup title={t("tts.voices.title")}>
      <div className="px-4 py-3 space-y-3">
        <p className="text-sm text-text/70">{t("tts.voices.description")}</p>
        {error && <p className="text-sm text-red-500 break-words">{error}</p>}

        <div className="space-y-1">
          {/* Die Standardstimme ist eine Stimme wie jede andere und wird
              gegen die anderen ausgewaehlt — das geht nur, wenn man sie auch
              hoeren kann. Leere Kennung heisst im Backend "Seed-Stimme". */}
          <div className="py-1">
            <div className="flex items-center justify-between gap-2 flex-wrap">
              <span className="text-sm">{t("tts.voices.defaultVoice")}</span>
              <div className="flex items-center gap-2">
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => void togglePreview(SEED_VOICE)}
                  aria-expanded={sample?.id === SEED_VOICE}
                  disabled={previewing !== null}
                >
                  <Play width={14} height={14} />
                  {previewing === SEED_VOICE
                    ? t("tts.voices.previewGenerating")
                    : t("tts.voices.preview")}
                </Button>
                {activeVoice === null ? (
                  <Badge variant="success">{t("tts.voices.active")}</Badge>
                ) : (
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => updateSetting("tts_voice", null)}
                  >
                    {t("tts.voices.activate")}
                  </Button>
                )}
              </div>
            </div>
            {sample?.id === SEED_VOICE &&
              (sample.data ? (
                <div className="mt-2 space-y-1">
                  <AudioPlayer
                    src={convertFileSrc(sample.data.wav_path, "asset")}
                    className="w-full"
                  />
                  <p className="text-xs text-text/60 italic">
                    {sample.data.transcript}
                  </p>
                </div>
              ) : (
                <p className="mt-2 text-xs text-red-400">
                  {sample.error ?? t("tts.voices.previewMissing")}
                </p>
              ))}
          </div>
          {voices.map(({ id, meta }) => (
            <div key={id} className="py-1">
              <div className="flex items-center justify-between gap-2 flex-wrap">
                <span className="text-sm font-medium">
                  {meta.display_name || id}
                </span>
                <div className="flex items-center gap-2">
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => void togglePreview(id)}
                    aria-expanded={sample?.id === id}
                    disabled={previewing !== null}
                  >
                    <Play width={14} height={14} />
                    {previewing === id
                      ? t("tts.voices.previewGenerating")
                      : t("tts.voices.preview")}
                  </Button>
                  {activeVoice === id ? (
                    <Badge variant="success">{t("tts.voices.active")}</Badge>
                  ) : (
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => updateSetting("tts_voice", id)}
                    >
                      {t("tts.voices.activate")}
                    </Button>
                  )}
                  {/* Der Stift oeffnet das Bearbeiten-Panel dieser Stimme:
                      Anzeigename, Farbe, Beschreibung, Tags, Klang — und die
                      Aktionen Umbenennen und Exportieren. */}
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() =>
                      setEditTarget((current) => (current === id ? null : id))
                    }
                    aria-expanded={editTarget === id}
                    title={t("tts.voiceEdit.open")}
                    aria-label={t("tts.voiceEdit.open")}
                  >
                    <Pencil width={14} height={14} />
                  </Button>
                  {/* Nur das Symbol: die Zeile traegt schon drei Knoepfe, und
                      der Papierkorb ist eindeutiger als ein viertes Wort.
                      Beschriftung wandert in title + aria-label. */}
                  <Button
                    size="sm"
                    variant="danger-ghost"
                    onClick={() => setDeleteTarget(id)}
                    title={t("tts.voices.delete")}
                    aria-label={t("tts.voices.delete")}
                  >
                    <Trash2 width={14} height={14} />
                  </Button>
                </div>
              </div>
              {sample?.id === id &&
                (sample.data ? (
                  <div className="mt-2 space-y-1">
                    {/* The same sentence for every voice — otherwise you are
                        comparing two recordings, not two voices. */}
                    <AudioPlayer
                      src={convertFileSrc(sample.data.wav_path, "asset")}
                      className="w-full"
                    />
                    <p className="text-xs text-text/60 italic">
                      {sample.data.transcript}
                    </p>
                  </div>
                ) : (
                  <p className="mt-2 text-xs text-red-400">
                    {sample.error ?? t("tts.voices.previewMissing")}
                  </p>
                ))}
              {editTarget === id && (
                <VoiceEditor
                  id={id}
                  onChanged={(newId) => void voiceEdited(newId)}
                />
              )}
            </div>
          ))}
          {voices.length === 0 && (
            <p className="text-sm text-text/60">{t("tts.voices.empty")}</p>
          )}
        </div>

        {mode.kind === "idle" && (
          <div className="flex flex-wrap gap-2">
            <Button onClick={startRecording}>{t("tts.voices.record")}</Button>
            <Button variant="secondary" onClick={pickFile}>
              <Upload width={14} height={14} />
              {t("tts.voices.import")}
            </Button>
            <Button
              variant="secondary"
              onClick={() => setBuilderOpen((open) => !open)}
              aria-expanded={builderOpen}
            >
              <Wand2 width={14} height={14} />
              {builderOpen ? t("common.close") : t("tts.builder.open")}
            </Button>
          </div>
        )}

        {/* Ein Archiv-Import gehoert zu keiner vorhandenen Stimme — er legt
            eine neue an und steht deshalb bei den Karten-Aktionen, nicht in
            einem Bearbeiten-Panel. */}
        {mode.kind === "idle" && (
          <VoiceArchiveImport onImported={(id) => void archiveImported(id)} />
        )}

        {mode.kind === "idle" && builderOpen && (
          <VoiceBuilder onSaved={(id) => void builderSaved(id)} />
        )}

        {mode.kind === "recording" && (
          <div className="flex items-center gap-3">
            <Badge variant="primary">
              {t("tts.voices.recording", { seconds: recordSeconds })}
            </Badge>
            <Button onClick={stopRecording} disabled={busy}>
              {t("tts.voices.stopRecording")}
            </Button>
            <span className="text-sm text-text/60">
              {t("tts.voices.recordingHint")}
            </span>
          </div>
        )}

        {mode.kind === "review" && (
          <div className="space-y-2">
            <Input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t("tts.voices.namePlaceholder")}
              className="w-full"
            />
            <Textarea
              value={transcript}
              onChange={(e) => setTranscript(e.target.value)}
              placeholder={
                transcribing
                  ? t("tts.voices.transcribing")
                  : !(getSetting("tts_reference_auto_transcribe") ?? true)
                    ? t("tts.voices.transcriptManualPlaceholder")
                    : mode.source === "recording"
                      ? t("tts.voices.transcriptPlaceholder")
                      : t("tts.voices.transcriptImportPlaceholder")
              }
              rows={4}
              className="w-full"
            />
            <div className="flex gap-2">
              <Button
                onClick={save}
                disabled={busy || transcribing || name.trim().length === 0}
              >
                {t("tts.voices.save")}
              </Button>
              <Button variant="ghost" onClick={discard} disabled={busy}>
                {t("tts.voices.discard")}
              </Button>
            </div>
          </div>
        )}
      </div>

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
        title={t("tts.voices.deleteConfirmTitle")}
        closeLabel={t("tts.voices.cancelDelete")}
        footer={
          <>
            <Button variant="secondary" onClick={() => setDeleteTarget(null)}>
              {t("tts.voices.cancelDelete")}
            </Button>
            <Button
              variant="danger"
              onClick={() => deleteTarget && void remove(deleteTarget)}
            >
              {t("tts.voices.delete")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {t("tts.voices.deleteConfirm", { voice: deleteTarget ?? "" })}
        </p>
      </Dialog>
    </SettingsGroup>
  );
};
