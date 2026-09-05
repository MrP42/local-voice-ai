import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Check, Download, Upload } from "lucide-react";
import { commands, type VoiceMeta } from "@/bindings";
import { VOICE_PALETTE, voiceColor } from "@/lib/voices/palette";
import { Button } from "../../../ui/Button";
import { Input } from "../../../ui/Input";
import { Textarea } from "../../../ui/Textarea";

/** Endung des Stimmen-Archivs — einmal hier, damit Speichern- und
 *  Oeffnen-Dialog nicht auseinanderlaufen koennen. */
const ARCHIVE_EXT = "lvvoice";

/** Grenzen der Klangregler, gespiegelt aus `VoiceSound` in den Bindings. */
const SPEED_MIN = 0.5;
const SPEED_MAX = 2.0;
const GAIN_MIN = -12;
const GAIN_MAX = 12;

/** Ein geworfener Fehler (Tauri liefert bei harten Fehlern eine Exception)
 *  soll denselben Weg gehen wie ein `Result`-Fehler: als Text in die Karte. */
const asMessage = (e: unknown): string =>
  e instanceof Error ? e.message : String(e);

/** Transportleiste und Sprecher-Chips halten eigene Listen; dieses Ereignis
 *  haelt sie mit der Registry zusammen, ohne dass sie sich kennen muessen. */
const notifyVoicesChanged = () =>
  window.dispatchEvent(new CustomEvent("lv-voices-changed"));

/** Tags stehen als eine Zeile im Formular — Komma trennt, Leerraum nicht. */
const parseTags = (raw: string): string[] =>
  raw
    .split(",")
    .map((tag) => tag.trim())
    .filter((tag) => tag.length > 0);

interface VoiceEditorProps {
  /** voice_id der Stimme, die bearbeitet wird. */
  id: string;
  /** Nach jeder Aenderung an der Registry: Liste in der Karte neu laden. */
  onChanged: (newId?: string) => void;
}

/**
 * Bearbeiten-Panel einer einzelnen Stimme: Anzeigename, Farbe, Beschreibung,
 * Default-Tags, Klangregler — dazu die beiden Aktionen, die keine Felder sind:
 * voice_id umbenennen (ein Umzug) und exportieren.
 */
export const VoiceEditor: React.FC<VoiceEditorProps> = ({ id, onChanged }) => {
  const { t } = useTranslation();
  const [meta, setMeta] = useState<VoiceMeta | null>(null);
  /** Der Anzeigename beim Oeffnen — Bezugspunkt der Marker-Warnung. */
  const [originalName, setOriginalName] = useState("");
  const [tagsRaw, setTagsRaw] = useState("");
  const [renameTo, setRenameTo] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setError(null);
    try {
      const res = await commands.ttsGetVoiceMeta(id);
      if (res.status === "error") {
        setError(res.error);
        return;
      }
      setMeta(res.data);
      setOriginalName(res.data.display_name);
      setTagsRaw(res.data.default_tags.join(", "));
      setRenameTo(res.data.display_name);
    } catch (e) {
      setError(asMessage(e));
    }
  }, [id]);

  useEffect(() => {
    void load();
  }, [load]);

  const patch = (change: Partial<VoiceMeta>) => {
    setNote(null);
    setMeta((current) => (current ? { ...current, ...change } : current));
  };

  const nameChanged =
    meta !== null && meta.display_name.trim() !== originalName.trim();

  const saveMeta = async () => {
    if (!meta) return;
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      const next: VoiceMeta = { ...meta, default_tags: parseTags(tagsRaw) };
      const res = await commands.ttsSetVoiceMeta(id, next);
      if (res.status === "error") {
        setError(res.error);
        return;
      }
      setMeta(next);
      setOriginalName(next.display_name);
      setTagsRaw(next.default_tags.join(", "));
      setNote(t("tts.voiceEdit.saved"));
      notifyVoicesChanged();
      onChanged();
    } catch (e) {
      setError(asMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const renameId = async () => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      const res = await commands.ttsRenameVoiceId(id, renameTo.trim());
      if (res.status === "error") {
        setError(res.error);
        return;
      }
      setNote(t("tts.voiceEdit.renameDone", { id: res.data }));
      notifyVoicesChanged();
      onChanged(res.data);
    } catch (e) {
      setError(asMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const exportVoice = async () => {
    setError(null);
    setNote(null);
    let target: string | null = null;
    try {
      target = await save({
        defaultPath: `${id}.${ARCHIVE_EXT}`,
        filters: [
          { name: t("tts.voiceEdit.archiveFilter"), extensions: [ARCHIVE_EXT] },
        ],
      });
    } catch (e) {
      setError(asMessage(e));
      return;
    }
    if (typeof target !== "string") return;
    setBusy(true);
    try {
      const res = await commands.ttsExportVoice(id, target);
      if (res.status === "error") {
        setError(res.error);
        return;
      }
      setNote(t("tts.voiceEdit.exportDone", { path: target }));
    } catch (e) {
      setError(asMessage(e));
    } finally {
      setBusy(false);
    }
  };

  if (!meta) {
    return (
      <div className="mt-2 rounded-lg border border-mid-gray/30 p-3">
        {error ? (
          <p className="text-sm text-red-400 break-words">{error}</p>
        ) : (
          <p className="text-sm text-text/60">{t("tts.voiceEdit.loading")}</p>
        )}
      </div>
    );
  }

  const sound = meta.sound ?? { speed: 1, gain_db: 0 };

  return (
    <div className="mt-2 space-y-4 rounded-lg border border-mid-gray/30 p-3">
      <p className="text-xs text-text/50">
        {t("tts.voiceEdit.currentId", { id })}
      </p>

      {error && <p className="text-sm text-red-400 break-words">{error}</p>}
      {note && <p className="text-sm text-text/70 break-words">{note}</p>}

      {/* 1. Anzeigename */}
      <div className="space-y-1">
        <label
          className="block text-sm font-medium"
          htmlFor={`voice-name-${id}`}
        >
          {t("tts.voiceEdit.nameLabel")}
        </label>
        <Input
          id={`voice-name-${id}`}
          type="text"
          value={meta.display_name}
          onChange={(e) => patch({ display_name: e.target.value })}
          className="w-full min-h-[44px]"
        />
      </div>

      {/* Die Warnung, die es wirklich gibt: Sprecher-Marker in gespeicherten
          Texten loesen ueber den Anzeigenamen auf (siehe
          `src/lib/voices/speakerMarkers.ts`). Wir blockieren nicht — wir
          sagen mit altem und neuem Namen, was danach nicht mehr greift. */}
      {nameChanged && (
        <div
          role="status"
          className="rounded-lg border border-amber-500/50 bg-amber-500/10 p-3 space-y-1"
        >
          <p className="text-sm font-semibold text-amber-500">
            {t("tts.voiceEdit.markerWarningTitle")}
          </p>
          <p className="text-sm text-text/80 break-words">
            {t("tts.voiceEdit.markerWarningBody", {
              old: originalName,
              new: meta.display_name.trim(),
            })}
          </p>
          <p className="text-xs text-text/70 break-words">
            {t("tts.voiceEdit.markerWarningExample")}{" "}
            <code className="font-mono">{`<${originalName}>`}</code>{" "}
            <code className="font-mono">{`${originalName}:`}</code>
          </p>
        </div>
      )}

      {/* 2. Farbe — die zehn Palette-Keys als Farbpunkte. */}
      <div className="space-y-1">
        <span className="block text-sm font-medium">
          {t("tts.voiceEdit.colorLabel")}
        </span>
        <div className="flex flex-wrap gap-2" role="radiogroup">
          {Object.keys(VOICE_PALETTE).map((key) => (
            <button
              key={key}
              type="button"
              role="radio"
              aria-checked={meta.color === key}
              aria-label={key}
              title={key}
              onClick={() => patch({ color: key })}
              className={`flex h-11 w-11 min-h-[44px] items-center justify-center rounded-full border-2 cursor-pointer transition-colors focus:outline-none focus:ring-2 focus:ring-logo-primary ${
                meta.color === key ? "border-text/80" : "border-transparent"
              }`}
            >
              <span
                className="flex h-7 w-7 items-center justify-center rounded-full"
                style={{ backgroundColor: voiceColor(key) }}
              >
                {meta.color === key && (
                  <Check width={14} height={14} className="text-white" />
                )}
              </span>
            </button>
          ))}
        </div>
      </div>

      {/* 3. Beschreibung und Default-Tags */}
      <div className="space-y-1">
        <label
          className="block text-sm font-medium"
          htmlFor={`voice-desc-${id}`}
        >
          {t("tts.voiceEdit.descriptionLabel")}
        </label>
        <Textarea
          id={`voice-desc-${id}`}
          value={meta.description ?? ""}
          onChange={(e) =>
            patch({
              description:
                e.target.value.trim().length > 0 ? e.target.value : null,
            })
          }
          placeholder={t("tts.voiceEdit.descriptionPlaceholder")}
          rows={3}
          className="w-full"
        />
      </div>

      <div className="space-y-1">
        <label
          className="block text-sm font-medium"
          htmlFor={`voice-tags-${id}`}
        >
          {t("tts.voiceEdit.tagsLabel")}
        </label>
        <Input
          id={`voice-tags-${id}`}
          type="text"
          value={tagsRaw}
          onChange={(e) => {
            setNote(null);
            setTagsRaw(e.target.value);
          }}
          placeholder={t("tts.voiceEdit.tagsPlaceholder")}
          className="w-full min-h-[44px]"
        />
        <p className="text-xs text-text/60">{t("tts.voiceEdit.tagsHint")}</p>
      </div>

      {/* 4. Klangregler — gelten bei JEDEM Vorlesen dieser Stimme. */}
      <div className="space-y-2">
        <p className="text-sm font-medium">{t("tts.voiceEdit.soundTitle")}</p>
        <div className="flex items-center gap-3">
          <label
            className="w-28 shrink-0 text-sm text-text/80"
            htmlFor={`voice-speed-${id}`}
          >
            {t("tts.voiceEdit.speedLabel")}
          </label>
          <input
            id={`voice-speed-${id}`}
            type="range"
            min={SPEED_MIN}
            max={SPEED_MAX}
            step={0.05}
            value={sound.speed}
            onChange={(e) =>
              patch({ sound: { ...sound, speed: parseFloat(e.target.value) } })
            }
            className="h-2 flex-grow cursor-pointer rounded-lg focus:outline-none focus:ring-2 focus:ring-logo-primary"
          />
          <span className="w-16 text-end text-sm font-medium tabular-nums">
            {t("tts.voiceEdit.speedValue", { value: sound.speed.toFixed(2) })}
          </span>
        </div>
        <div className="flex items-center gap-3">
          <label
            className="w-28 shrink-0 text-sm text-text/80"
            htmlFor={`voice-gain-${id}`}
          >
            {t("tts.voiceEdit.gainLabel")}
          </label>
          <input
            id={`voice-gain-${id}`}
            type="range"
            min={GAIN_MIN}
            max={GAIN_MAX}
            step={0.5}
            value={sound.gain_db}
            onChange={(e) =>
              patch({
                sound: { ...sound, gain_db: parseFloat(e.target.value) },
              })
            }
            className="h-2 flex-grow cursor-pointer rounded-lg focus:outline-none focus:ring-2 focus:ring-logo-primary"
          />
          <span className="w-16 text-end text-sm font-medium tabular-nums">
            {t("tts.voiceEdit.gainValue", {
              value: `${sound.gain_db > 0 ? "+" : ""}${sound.gain_db.toFixed(1)}`,
            })}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="ghost"
            onClick={() => patch({ sound: null })}
            disabled={meta.sound === null}
          >
            {t("tts.voiceEdit.soundReset")}
          </Button>
          {meta.sound === null && (
            <span className="text-xs text-text/60">
              {t("tts.voiceEdit.soundUnchanged")}
            </span>
          )}
        </div>
      </div>

      <div className="flex flex-wrap gap-2">
        <Button
          className="min-h-[44px]"
          onClick={() => void saveMeta()}
          disabled={busy || meta.display_name.trim().length === 0}
        >
          {busy ? t("tts.voiceEdit.saving") : t("tts.voiceEdit.save")}
        </Button>
        <Button
          className="min-h-[44px]"
          variant="secondary"
          onClick={() => void exportVoice()}
          disabled={busy}
        >
          <Download width={14} height={14} />
          {t("tts.voiceEdit.export")}
        </Button>
      </div>

      {/* 5. voice_id umbenennen — ausdrueckliche Aktion, kein Nebeneffekt des
          Anzeigenamens: die id ist Ordnername, Einstellungswert und Teil des
          Cache-Schluessels. */}
      <div className="space-y-2 border-t border-mid-gray/30 pt-3">
        <p className="text-sm font-medium">{t("tts.voiceEdit.renameTitle")}</p>
        <p className="text-xs text-text/60">{t("tts.voiceEdit.renameHint")}</p>
        <div className="flex flex-wrap items-center gap-2">
          <Input
            type="text"
            aria-label={t("tts.voiceEdit.renameLabel")}
            value={renameTo}
            onChange={(e) => {
              setNote(null);
              setRenameTo(e.target.value);
            }}
            placeholder={t("tts.voiceEdit.renamePlaceholder")}
            className="min-h-[44px] flex-grow"
          />
          <Button
            className="min-h-[44px]"
            variant="secondary"
            onClick={() => void renameId()}
            disabled={busy || renameTo.trim().length === 0}
          >
            {t("tts.voiceEdit.renameAction")}
          </Button>
        </div>
      </div>
    </div>
  );
};

interface VoiceImportProps {
  /** Nach dem Import: Liste neu laden und die frische Stimme melden. */
  onImported: (voiceId: string) => void;
}

/**
 * Importieren eines `.lvvoice`-Archivs. Steht neben den anderen Karten-
 * Aktionen statt in einem Stimmen-Panel, weil ein Import zu keiner
 * vorhandenen Stimme gehoert — er legt eine neue an. Erst Vorschau
 * (`tts_inspect_voice_archive`), dann einspielen.
 */
export const VoiceArchiveImport: React.FC<VoiceImportProps> = ({
  onImported,
}) => {
  const { t } = useTranslation();
  const [preview, setPreview] = useState<{ path: string; name: string } | null>(
    null,
  );
  const [nameOverride, setNameOverride] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const pick = async () => {
    setError(null);
    setPreview(null);
    let picked: string | string[] | null = null;
    try {
      picked = await open({
        multiple: false,
        filters: [
          { name: t("tts.voiceEdit.archiveFilter"), extensions: [ARCHIVE_EXT] },
        ],
      });
    } catch (e) {
      setError(asMessage(e));
      return;
    }
    if (typeof picked !== "string") return;
    setBusy(true);
    try {
      const res = await commands.ttsInspectVoiceArchive(picked);
      if (res.status === "error") {
        setError(res.error);
        return;
      }
      setPreview({ path: picked, name: res.data.display_name });
      setNameOverride(res.data.display_name);
    } catch (e) {
      setError(asMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const confirmImport = async () => {
    if (!preview) return;
    setBusy(true);
    setError(null);
    try {
      const trimmed = nameOverride.trim();
      const res = await commands.ttsImportVoiceArchive(
        preview.path,
        trimmed.length > 0 && trimmed !== preview.name ? trimmed : null,
      );
      if (res.status === "error") {
        setError(res.error);
        return;
      }
      setPreview(null);
      notifyVoicesChanged();
      onImported(res.data);
    } catch (e) {
      setError(asMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-2">
      <Button
        className="min-h-[44px]"
        variant="secondary"
        onClick={() => void pick()}
        disabled={busy}
      >
        <Upload width={14} height={14} />
        {t("tts.voiceEdit.import")}
      </Button>
      {error && <p className="text-sm text-red-400 break-words">{error}</p>}
      {preview && (
        <div className="space-y-2 rounded-lg border border-mid-gray/30 p-3">
          <p className="text-sm text-text/80 break-words">
            {t("tts.voiceEdit.importPreview", { name: preview.name })}
          </p>
          <Input
            type="text"
            aria-label={t("tts.voiceEdit.importNameLabel")}
            value={nameOverride}
            onChange={(e) => setNameOverride(e.target.value)}
            className="w-full min-h-[44px]"
          />
          <div className="flex flex-wrap gap-2">
            <Button
              className="min-h-[44px]"
              onClick={() => void confirmImport()}
              disabled={busy}
            >
              {busy
                ? t("tts.voiceEdit.importing")
                : t("tts.voiceEdit.importConfirm")}
            </Button>
            <Button
              className="min-h-[44px]"
              variant="ghost"
              onClick={() => setPreview(null)}
              disabled={busy}
            >
              {t("tts.voiceEdit.importCancel")}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
};
