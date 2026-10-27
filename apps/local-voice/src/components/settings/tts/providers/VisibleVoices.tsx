import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Checkbox } from "@fluentui/react-components";
import {
  commands,
  type TtsProviderEntry,
  type VoiceDescriptor,
} from "@/bindings";
import { Button } from "../../../ui/Button";
import {
  PROVIDER_DEFAULT_VOICE,
  asTtsError,
  isVoiceShown,
  toggleVisibleVoice,
} from "./providerModel";

/** Fensterereignis: die Auswahl "im Vorlesen anzeigen" hat sich geaendert (Dropdowns laden neu). */
export const PROVIDER_VOICES_CHANGED = "lv-provider-voices-changed";

/**
 * Liste aller Stimmen eines Anbieters mit Haken "im Vorlesen anzeigen": Name,
 * Geschlecht und Kurzbeschreibung je Stimme, dazu "Alle" und "Keine". Die
 * Auswahl gilt fuer das Dropdown der Vorlesen-Seite und die Wiedergabeleiste
 * der Aufnahmen. Die gerade gewaehlte Vorlese-Stimme bleibt sichtbar (Haken
 * gesetzt und gesperrt), die Standardstimme steht nie in der Liste. Nichts hier
 * kennt einen bestimmten Anbieter: gezeigt wird, was die Stimmenliste liefert.
 */
export const VisibleVoices: React.FC<{ entry: TtsProviderEntry }> = ({
  entry,
}) => {
  const { t } = useTranslation();
  const id = entry.descriptor.id;
  const [voices, setVoices] = useState<VoiceDescriptor[] | null>(null);
  const [visible, setVisible] = useState<string[]>(entry.visible_voices ?? []);
  const [error, setError] = useState<string | null>(null);
  const visibleKey = (entry.visible_voices ?? []).join("\u0000");

  // Der Stand des Backends gilt, sobald die Liste neu geladen wurde.
  useEffect(() => {
    setVisible(entry.visible_voices ?? []);
    // visibleKey statt des Arrays: nur ein geaenderter Inhalt zaehlt.
  }, [visibleKey]);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      void commands
        .ttsProviderVoices(id, null)
        .then((result) => {
          if (cancelled) return;
          if (result.status === "ok") setVoices(result.data);
          else setVoices([]);
        })
        .catch(() => {
          if (!cancelled) setVoices([]);
        });
    };
    load();
    window.addEventListener("lv-voices-changed", load);
    return () => {
      cancelled = true;
      window.removeEventListener("lv-voices-changed", load);
    };
  }, [id]);

  const named = useMemo(
    () => (voices ?? []).filter((v) => v.id !== PROVIDER_DEFAULT_VOICE),
    [voices],
  );
  const allIds = useMemo(() => named.map((v) => v.id), [named]);
  const selected = entry.voice;
  const shownCount = named.filter((v) =>
    isVoiceShown(visible, selected, v.id),
  ).length;

  const persist = useCallback(
    async (next: string[]) => {
      setError(null);
      const before = visible;
      setVisible(next);
      const result = await commands
        .ttsProviderSetVisibleVoices(id, next)
        .catch((e) => ({ status: "error" as const, error: asTtsError(e) }));
      if (result.status === "ok") {
        window.dispatchEvent(new CustomEvent(PROVIDER_VOICES_CHANGED));
      } else {
        setVisible(before);
        setError(result.error.message);
      }
    },
    [id, visible],
  );

  if (voices === null || named.length === 0) {
    return (
      <div
        className="space-y-1"
        data-testid={`tts-provider-visible-${id}`}
        aria-busy={voices === null}
      >
        <h4 className="text-xs font-medium text-text/80">
          {t("tts.providers.voices.visible.title")}
        </h4>
        {voices !== null && (
          <p className="text-xs text-text/70">
            {t("tts.providers.voices.visible.empty")}
          </p>
        )}
      </div>
    );
  }

  return (
    <div className="space-y-2" data-testid={`tts-provider-visible-${id}`}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h4 className="text-xs font-medium text-text/80">
          {t("tts.providers.voices.visible.title")}
        </h4>
        <div className="flex items-center gap-2">
          <span
            className="text-xs tabular-nums text-text/70"
            data-testid={`tts-provider-visible-count-${id}`}
          >
            {t("tts.providers.voices.visible.count", {
              shown: shownCount,
              total: named.length,
            })}
          </span>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            data-testid={`tts-provider-visible-all-${id}`}
            onClick={() => void persist([])}
          >
            {t("tts.providers.voices.visible.all")}
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            data-testid={`tts-provider-visible-none-${id}`}
            onClick={() => void persist([PROVIDER_DEFAULT_VOICE])}
          >
            {t("tts.providers.voices.visible.none")}
          </Button>
        </div>
      </div>
      <p className="text-xs text-text/70">
        {t("tts.providers.voices.visible.hint")}
      </p>
      <ul
        className="max-h-72 divide-y divide-mid-gray/15 overflow-y-auto rounded-md border border-mid-gray/20"
        data-testid={`tts-provider-visible-list-${id}`}
      >
        {named.map((voice) => {
          const locked = voice.id === selected;
          const shown = isVoiceShown(visible, selected, voice.id);
          const [name, ...rest] = voice.label.split(" – ");
          return (
            <li
              key={voice.id}
              className="flex items-start gap-2 px-3 py-1.5"
              data-voice-id={voice.id}
            >
              <Checkbox
                checked={shown}
                disabled={locked}
                onChange={(_e, data) =>
                  void persist(
                    toggleVisibleVoice(
                      visible,
                      allIds,
                      voice.id,
                      data.checked === true,
                    ),
                  )
                }
                aria-label={t("tts.providers.voices.visible.toggle", {
                  name: name.trim() || voice.id,
                })}
                data-testid={`tts-provider-visible-check-${voice.id}`}
              />
              <div className="min-w-0 flex-1 text-sm">
                <div className="flex flex-wrap items-baseline gap-x-2">
                  <span className="font-medium text-text">
                    {name.trim() || voice.id}
                  </span>
                  {voice.gender && (
                    <span className="text-xs text-text/70">
                      {t(`tts.providers.voices.gender.${voice.gender}`, {
                        defaultValue: voice.gender,
                      })}
                    </span>
                  )}
                  {locked && (
                    <span className="text-xs text-text/70">
                      {t("tts.providers.voices.visible.current")}
                    </span>
                  )}
                </div>
                {rest.length > 0 && (
                  <p className="text-xs text-text/70">{rest.join(" – ")}</p>
                )}
              </div>
            </li>
          );
        })}
      </ul>
      {error && (
        <p
          role="alert"
          className="text-xs text-red-500"
          data-testid={`tts-provider-visible-error-${id}`}
        >
          {error}
        </p>
      )}
    </div>
  );
};
