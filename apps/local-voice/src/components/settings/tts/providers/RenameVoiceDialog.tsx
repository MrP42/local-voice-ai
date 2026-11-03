import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pencil } from "lucide-react";
import { commands } from "@/bindings";
import { useSettings } from "../../../../hooks/useSettings";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { Input } from "../../../ui/Input";
import { asTtsError } from "./providerModel";
import { PROVIDER_VOICES_CHANGED } from "./VisibleVoices";

/**
 * Was umbenannt wird. Anbieter (Phonon, Sprachserver, Ollama, Piper): eigener
 * Anzeigename in `tts_providers.voice_names`, die Kennung bleibt. Fish: der
 * Anzeigename in den Metadaten der eigenen Stimme (`display_name`).
 */
export type RenameVoiceTarget = {
  kind: "provider" | "fish";
  /** Anbieter-Kennung (`phonon`, `piper`, …); bei Fish `fish`. */
  providerId: string;
  /** Kennung der Stimme beim Anbieter (bleibt immer gleich). */
  voiceId: string;
  /** Der Name, der jetzt angezeigt wird. */
  name: string;
  /** Der Name stammt vom Nutzer (nur Anbieter: dann gibt es „Ursprünglicher Name“). */
  custom: boolean;
  /** Namen und Kennungen der anderen Stimmen desselben Anbieters (Marker müssen eindeutig bleiben). */
  taken?: string[];
};

/** Vergleichsform eines Namens wie im Backend (`fold_speaker_name`): klein, Umlaute ausgeschrieben. */
export function foldVoiceName(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/ä/g, "ae")
    .replace(/ö/g, "oe")
    .replace(/ü/g, "ue")
    .replace(/ß/g, "ss")
    .replace(/\s+/g, " ");
}

/** Längster Name (wie ein Sprecher-Marker `<Name>`, siehe `policy::clean_voice_name`). */
export const MAX_VOICE_NAME = 60;

/** Fehlertext-Schlüssel für einen ungültigen Namen, sonst `null` (Spiegel der Regel im Backend). */
export function voiceNameProblem(
  name: string,
  taken: readonly string[] = [],
): string | null {
  const clean = name.trim().replace(/\s+/g, " ");
  if ([...clean].length > MAX_VOICE_NAME) return "tooLong";
  if (/[<>:]/.test(clean)) return "badChars";
  if (clean.includes(" – ") || clean.includes(" · ")) return "dash";
  if (clean && taken.some((n) => foldVoiceName(n) === foldVoiceName(clean)))
    return "taken";
  return null;
}

/** Nach einer Umbenennung: alle Listen (Dropdown, Katalog, Sprecher) neu lesen. */
function announceRename() {
  window.dispatchEvent(new CustomEvent("lv-voices-changed"));
  window.dispatchEvent(new CustomEvent(PROVIDER_VOICES_CHANGED));
}

async function saveName(
  target: RenameVoiceTarget,
  name: string | null,
): Promise<string | null> {
  if (target.kind === "fish") {
    const infos = await commands.ttsListVoiceInfos().catch(() => []);
    const info = infos.find((v) => v.id === target.voiceId);
    if (!info) return "unknown";
    const result = await commands.ttsSetVoiceMeta(target.voiceId, {
      ...info.meta,
      display_name: name ?? target.voiceId,
    });
    return result.status === "ok" ? null : result.error;
  }
  const result = await commands
    .ttsProviderSetVoiceName(
      target.providerId,
      target.voiceId,
      name,
      target.custom ? null : target.name,
    )
    .catch((e) => ({ status: "error" as const, error: asTtsError(e) }));
  return result.status === "ok" ? null : result.error.message;
}

/**
 * Dialog „Stimme umbenennen“: ein Feld für den Anzeigenamen. Der neue Name steht
 * danach in der Vorlesen-Auswahl, in der Wiedergabeleiste, im Stimmenkatalog und
 * als Sprecher im Skript; die Stimme selbst (Kennung, Klang, Hörproben) bleibt.
 */
export const RenameVoiceDialog: React.FC<{
  target: RenameVoiceTarget | null;
  onOpenChange: (open: boolean) => void;
}> = ({ target, onOpenChange }) => {
  const { t } = useTranslation();
  const { refreshSettings } = useSettings();
  const field = useRef<HTMLInputElement>(null);
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!target) return;
    setValue(target.name);
    setError(null);
    setBusy(false);
  }, [target]);

  if (!target) return null;

  const problem = voiceNameProblem(value, target.taken);
  const unchanged = value.trim().replace(/\s+/g, " ") === target.name;

  const apply = async (name: string | null) => {
    setBusy(true);
    setError(null);
    const failed = await saveName(target, name);
    setBusy(false);
    if (failed) {
      setError(
        failed === "unknown"
          ? t("tts.providers.voices.rename.problem.unknown")
          : failed,
      );
      return;
    }
    await refreshSettings();
    announceRename();
    onOpenChange(false);
  };

  const save = () => {
    if (problem || busy) return;
    if (unchanged) {
      onOpenChange(false);
      return;
    }
    const clean = value.trim().replace(/\s+/g, " ");
    void apply(clean === "" ? null : clean);
  };

  return (
    <Dialog
      open
      onOpenChange={onOpenChange}
      title={t("tts.providers.voices.rename.title")}
      description={t("tts.providers.voices.rename.description")}
      closeLabel={t("tts.providers.voices.rename.cancel")}
      className="max-w-md"
      contentFades={false}
      initialFocusRef={field}
      footer={
        <>
          <Button
            variant="primary"
            onClick={save}
            disabled={busy || problem !== null}
            data-testid="voice-rename-save"
          >
            {t("tts.providers.voices.rename.save")}
          </Button>
          {target.kind === "provider" && target.custom && (
            <Button
              variant="secondary"
              onClick={() => void apply(null)}
              disabled={busy}
              data-testid="voice-rename-reset"
            >
              {t("tts.providers.voices.rename.reset")}
            </Button>
          )}
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("tts.providers.voices.rename.cancel")}
          </Button>
        </>
      }
    >
      {/* Ein einziges Textfeld: Enter speichert (implizites Absenden). */}
      <form
        className="space-y-2"
        data-testid="voice-rename-dialog"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <label className="block space-y-1">
          <span className="text-sm">
            {t("tts.providers.voices.rename.label")}
          </span>
          <Input
            ref={field}
            value={value}
            maxLength={MAX_VOICE_NAME + 10}
            onChange={(e) => setValue(e.target.value)}
            onFocus={(e) => e.target.select()}
            aria-invalid={problem !== null}
            data-testid="voice-rename-input"
            className="w-full"
          />
        </label>
        <p className="text-xs text-text/70" data-testid="voice-rename-id">
          {target.kind === "fish"
            ? t("tts.providers.voices.rename.idHint", { id: target.voiceId })
            : t("tts.providers.voices.rename.providerHint", {
                id: target.voiceId,
              })}
        </p>
        {(problem || error) && (
          <p
            role="alert"
            className="text-xs text-red-500"
            data-testid="voice-rename-error"
          >
            {problem
              ? t(`tts.providers.voices.rename.problem.${problem}`, {
                  max: MAX_VOICE_NAME,
                })
              : error}
          </p>
        )}
      </form>
    </Dialog>
  );
};

/** Stift-Knopf „Umbenennen“ mit eigenem Dialog. 44 px auf Touch, 32 px am Desktop. */
export const RenameVoiceButton: React.FC<{
  target: RenameVoiceTarget;
  testId?: string;
  compact?: boolean;
}> = ({ target, testId, compact = false }) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState<RenameVoiceTarget | null>(null);
  const label = t("tts.providers.voices.rename.button", { name: target.name });
  return (
    <>
      <button
        type="button"
        aria-label={label}
        title={label}
        data-testid={
          testId ?? `voice-rename-${target.providerId}-${target.voiceId}`
        }
        onClick={() => setOpen(target)}
        className={`inline-flex shrink-0 items-center justify-center rounded-md text-text/60 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-2 focus-visible:outline-logo-primary ${
          compact
            ? "h-[44px] w-[44px] md:h-7 md:w-7"
            : "h-[44px] w-[44px] md:h-8 md:w-8"
        }`}
      >
        <Pencil className="h-4 w-4" aria-hidden="true" />
      </button>
      <RenameVoiceDialog
        target={open}
        onOpenChange={(o) => {
          if (!o) setOpen(null);
        }}
      />
    </>
  );
};
