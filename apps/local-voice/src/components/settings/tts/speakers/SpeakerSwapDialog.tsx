import React, { useEffect, useMemo, useState } from "react";
import { Select as FluentSelect } from "@fluentui/react-components";
import { useTranslation } from "react-i18next";
import { Dialog } from "../../../ui/Dialog";
import { Button } from "../../../ui/Button";
import {
  speaksLanguage,
  type LangVoice,
} from "@/lib/voices/speakerSwap";
import type { SpeakerRef } from "@/lib/voices/speakerMarkers";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Die Sprecher der Übersetzung (aus dem Original übernommen). */
  sources: SpeakerRef[];
  /** Alle Stimmen mit Sprache. */
  voices: LangVoice[];
  /** Anzeigename der Zielsprache (`English`) und ihr Kürzel (`en`). */
  targetLang: string;
  targetCode: string;
  /** Vorbelegung: Sprecher-id → Stimmen-id. */
  initial: Record<string, string>;
  onApply: (mapping: Record<string, string>) => void;
}

/**
 * Sprecher einer Übersetzung tauschen: je Sprecher des Originals eine Stimme,
 * Stimmen der Zielsprache zuerst. Vorbelegt mit dem Vorschlag; „Übernehmen"
 * ersetzt die Marken im übersetzten Text.
 */
export const SpeakerSwapDialog: React.FC<Props> = ({
  open,
  onOpenChange,
  sources,
  voices,
  targetLang,
  targetCode,
  initial,
  onApply,
}) => {
  const { t } = useTranslation();
  const [mapping, setMapping] = useState<Record<string, string>>(initial);
  useEffect(() => {
    if (open) setMapping(initial);
  }, [open, initial]);

  const ordered = useMemo(() => {
    const fits = voices.filter((v) => speaksLanguage(v, targetCode));
    const rest = voices.filter((v) => !speaksLanguage(v, targetCode));
    return { fits, rest };
  }, [voices, targetCode]);

  const label = (v: LangVoice) =>
    v.language ? `${v.displayName} · ${v.language}` : v.displayName;

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("tts.speakerSwap.title", { lang: targetLang })}
      description={
        ordered.fits.length > 0
          ? t("tts.speakerSwap.description", { lang: targetLang })
          : t("tts.speakerSwap.noVoices", { lang: targetLang })
      }
      closeLabel={t("tts.speakerSwap.cancel")}
      className="max-w-lg"
      footer={
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("tts.speakerSwap.cancel")}
          </Button>
          <Button
            onClick={() => {
              onApply(mapping);
              onOpenChange(false);
            }}
            data-testid="speaker-swap-apply"
          >
            {t("tts.speakerSwap.apply")}
          </Button>
        </div>
      }
    >
      <ul className="space-y-2" data-testid="speaker-swap-list">
        {sources.map((source) => (
          <li key={source.id} className="flex items-center gap-3 text-sm">
            <span
              className="min-w-0 flex-1 truncate rounded px-1.5 py-0.5"
              style={{ background: `${source.color}33` }}
            >
              {source.displayName}
            </span>
            <span aria-hidden="true">→</span>
            <FluentSelect
              value={mapping[source.id] ?? source.id}
              onChange={(e) =>
                setMapping((m) => ({ ...m, [source.id]: e.target.value }))
              }
              aria-label={t("tts.speakerSwap.voiceFor", {
                name: source.displayName,
              })}
              data-testid={`speaker-swap-${source.id}`}
              className="w-56"
            >
              {ordered.fits.length > 0 && (
                <optgroup label={targetLang}>
                  {ordered.fits.map((v) => (
                    <option key={v.id} value={v.id}>
                      {label(v)}
                    </option>
                  ))}
                </optgroup>
              )}
              <optgroup label={t("tts.speakerSwap.otherVoices")}>
                {ordered.rest.map((v) => (
                  <option key={v.id} value={v.id}>
                    {label(v)}
                  </option>
                ))}
              </optgroup>
            </FluentSelect>
          </li>
        ))}
      </ul>
    </Dialog>
  );
};
