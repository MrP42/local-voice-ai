import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight, Trash2, Users } from "lucide-react";
import { commands } from "@/bindings";
import { Input } from "@/components/ui/Input";
import { voiceColor } from "@/lib/voices/palette";
import {
  scanSpeakerMarkers,
  speakerMarkerText,
  type SpeakerRef,
} from "@/lib/voices/speakerMarkers";
import type {
  ChipMatch,
  ChipMenuApi,
  ChipPopoverApi,
  ChipProvider,
} from "../editor/TtsChipEditor";

/**
 * Alle Stimmen als Sprecher — Anzeigename und Farbe kommen aus der
 * `meta.json` je Stimme (Registry), nicht aus der id. Die Verwaltung meldet
 * Änderungen über dasselbe Fensterereignis wie die Stimmenliste, damit
 * Chips und Verwaltung nie auseinanderlaufen.
 */
export function useSpeakers(): SpeakerRef[] {
  const [speakers, setSpeakers] = useState<SpeakerRef[]>([]);
  useEffect(() => {
    let alive = true;
    const load = () => {
      void commands
        .ttsListVoiceInfos()
        .then((infos) => {
          if (!alive) return;
          setSpeakers(
            infos.map((info) => ({
              id: info.id,
              displayName: info.meta.display_name,
              color: voiceColor(info.meta.color),
            })),
          );
        })
        .catch(() => undefined);
    };
    load();
    window.addEventListener("lv-voices-changed", load);
    return () => {
      alive = false;
      window.removeEventListener("lv-voices-changed", load);
    };
  }, []);
  return speakers;
}

/**
 * Ab so vielen Einträgen bekommt eine selbstgebaute Liste ein Suchfeld. Bei
 * drei Stimmen wäre es nur im Weg, bei dreißig ist Scrollen die Zumutung.
 */
const SEARCH_THRESHOLD = 8;

/**
 * Suchfeld und gefilterte Liste für die Sprecherlisten. Gefiltert wird über
 * Teilzeichenketten mit `toLowerCase()` — nicht `localeCompare`, das kennt
 * keine Teiltreffer; so gehen Umlaute mit.
 */
const useSpeakerSearch = (speakers: SpeakerRef[]) => {
  const [query, setQuery] = useState("");
  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (needle === "") return speakers;
    return speakers.filter((speaker) =>
      speaker.displayName.toLowerCase().includes(needle),
    );
  }, [query, speakers]);
  return {
    query,
    setQuery,
    filtered,
    showSearch: speakers.length > SEARCH_THRESHOLD,
  };
};

/** Das Suchfeld selbst — beim Öffnen der Liste bekommt es den Fokus. */
const SpeakerSearchField: React.FC<{
  value: string;
  onChange: (value: string) => void;
}> = ({ value, onChange }) => {
  const { t } = useTranslation();
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    inputRef.current?.focus();
  }, []);
  return (
    <Input
      ref={inputRef}
      type="text"
      variant="compact"
      value={value}
      onChange={(event) => onChange(event.target.value)}
      placeholder={t("tts.speakers.searchPlaceholder")}
      aria-label={t("tts.speakers.searchPlaceholder")}
      className="w-full"
    />
  );
};

/** Ein Sprecher in einer Liste: Farbpunkt + Name, 44 px hoch (Touch). */
const SpeakerRow: React.FC<{
  speaker: SpeakerRef;
  current?: boolean;
  onPick: (speaker: SpeakerRef) => void;
}> = ({ speaker, current = false, onPick }) => (
  <button
    type="button"
    role="menuitem"
    aria-current={current || undefined}
    onClick={() => onPick(speaker)}
    className={`flex min-h-[44px] w-full cursor-pointer items-center gap-2 rounded-md px-2 text-start text-sm hover:bg-mid-gray/15 hover:text-text focus-visible:bg-mid-gray/15 focus-visible:text-text focus-visible:outline-none ${
      current ? "text-text" : "text-text/80"
    }`}
  >
    <span
      aria-hidden="true"
      className="size-2.5 shrink-0 rounded-full"
      style={{ backgroundColor: speaker.color }}
    />
    <span className="min-w-0 flex-1 truncate">{speaker.displayName}</span>
    <span className="shrink-0 text-xs text-text/45">
      {speakerMarkerText(speaker)}
    </span>
  </button>
);

/**
 * Popover eines Sprecher-Chips: der aktuelle Sprecher oben, darunter alle
 * anderen zur Auswahl — ein Klick hängt die Stelle auf eine andere Stimme
 * um. Ein vorhandener Stil (`<Anna:fluesternd>`) bleibt dabei erhalten.
 */
const SpeakerChipPopover: React.FC<{
  match: ChipMatch;
  api: ChipPopoverApi;
  speakers: SpeakerRef[];
}> = ({ match, api, speakers }) => {
  const { t } = useTranslation();
  const { query, setQuery, filtered, showSearch } = useSpeakerSearch(speakers);
  const marker = scanSpeakerMarkers(match.raw, speakers)[0];
  const current = marker?.speaker;
  const style = marker?.style;

  const replaceWith = (speaker: SpeakerRef) => {
    api.replaceRange(match.start, match.end, speakerMarkerText(speaker, style));
    api.close();
  };

  const remove = () => {
    api.replaceRange(match.start, match.end, "");
    api.close();
  };

  return (
    <div className="w-64" role="menu" aria-label={t("tts.speakers.change")}>
      <div className="flex items-center gap-2 border-b border-mid-gray/15 px-3 py-2">
        {current && (
          <span
            aria-hidden="true"
            className="size-2.5 shrink-0 rounded-full"
            style={{ backgroundColor: current.color }}
          />
        )}
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-text">
          {current?.displayName ?? match.raw}
        </span>
        <button
          type="button"
          onClick={remove}
          title={t("tts.speakers.remove")}
          aria-label={t("tts.speakers.remove")}
          className="flex min-h-[44px] min-w-[44px] cursor-pointer items-center justify-center rounded-md text-text/50 transition-colors hover:bg-mid-gray/15 hover:text-red-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
        >
          <Trash2 width={15} height={15} aria-hidden="true" />
        </button>
      </div>
      <p className="px-3 pt-2 text-[11px] leading-4 text-text/45">
        {t("tts.speakers.changeHint")}
      </p>
      {showSearch && (
        <div className="px-3 pt-2">
          <SpeakerSearchField value={query} onChange={setQuery} />
        </div>
      )}
      <div className="max-h-64 overflow-y-auto p-1">
        {filtered.length === 0 ? (
          <p className="px-2 py-3 text-xs text-text/50">
            {t("tts.speakers.emptySearch")}
          </p>
        ) : (
          filtered.map((speaker) => (
            <SpeakerRow
              key={speaker.id}
              speaker={speaker}
              current={speaker.id === current?.id}
              onPick={replaceWith}
            />
          ))
        )}
      </div>
    </div>
  );
};

/** Abschnitt „Sprecher einfügen" im Kontextmenü des Editors. */
const SpeakerMenuSection: React.FC<{
  api: ChipMenuApi;
  speakers: SpeakerRef[];
}> = ({ api, speakers }) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const { query, setQuery, filtered, showSearch } = useSpeakerSearch(speakers);
  return (
    <>
      <div className="my-1 border-t border-mid-gray/15" aria-hidden="true" />
      <button
        type="button"
        role="menuitem"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex min-h-[44px] w-full cursor-pointer items-center gap-2 px-3 text-start text-sm text-text/80 hover:bg-mid-gray/15 hover:text-text focus-visible:bg-mid-gray/15 focus-visible:text-text focus-visible:outline-none"
      >
        <Users width={15} height={15} aria-hidden="true" />
        <span className="flex-1">{t("tts.speakers.menuInsert")}</span>
        {open ? (
          <ChevronDown width={15} height={15} aria-hidden="true" />
        ) : (
          <ChevronRight width={15} height={15} aria-hidden="true" />
        )}
      </button>
      {open && (
        <div className="border-t border-mid-gray/15 px-2 py-1">
          {speakers.length === 0 ? (
            <p className="px-2 py-3 text-xs text-text/50">
              {t("tts.speakers.empty")}
            </p>
          ) : (
            <>
              {showSearch && (
                <div className="px-1 pb-1">
                  <SpeakerSearchField value={query} onChange={setQuery} />
                </div>
              )}
              {filtered.length === 0 ? (
                <p className="px-2 py-3 text-xs text-text/50">
                  {t("tts.speakers.emptySearch")}
                </p>
              ) : (
                <div className="max-h-64 overflow-y-auto">
                  {filtered.map((speaker) => (
                    <SpeakerRow
                      key={speaker.id}
                      speaker={speaker}
                      onPick={(picked) =>
                        api.insertAtSelection(`${speakerMarkerText(picked)} `)
                      }
                    />
                  ))}
                </div>
              )}
            </>
          )}
        </div>
      )}
    </>
  );
};

/**
 * Der Sprecher-Provider für den `TtsChipEditor` — das Gegenstück zum
 * Tag-Provider, gegen denselben `ChipProvider`-Vertrag.
 *
 * Ein Chip entsteht NUR für Marker, die auch das Backend schaltet (Abgleich
 * gegen die Stimmen-Registry). Ein unbekanntes `<div>` bleibt deshalb
 * gewöhnlicher Text — und wird, wie im Backend, wörtlich mitgelesen; es als
 * Fehler zu markieren würde eingefügtes HTML das Vorlesen blockieren lassen.
 *
 * Jeder Chip eröffnet zugleich eine Strecke (`rangeColor`): der Text bis zum
 * nächsten Sprecherwechsel bekommt die Farbe der Stimme als blassen
 * Hintergrund.
 */
export function useSpeakerProvider(): ChipProvider {
  const { t } = useTranslation();
  const speakers = useSpeakers();

  const scan = useCallback(
    (text: string): ChipMatch[] =>
      scanSpeakerMarkers(text, speakers).map((m) => ({
        start: m.start,
        end: m.end,
        raw: m.raw,
      })),
    [speakers],
  );

  return useMemo<ChipProvider>(
    () => ({
      id: "speaker",
      scan,
      render: (m) => {
        const marker = scanSpeakerMarkers(m.raw, speakers)[0];
        const color = marker?.speaker.color;
        return {
          label: marker
            ? t("tts.speakers.chipTitle", { name: marker.speaker.displayName })
            : m.raw,
          color,
          rangeColor: color,
          state: "ok",
        };
      },
      popover: (m, api) => (
        <SpeakerChipPopover match={m} api={api} speakers={speakers} />
      ),
      menuSection: (api) => (
        <SpeakerMenuSection api={api} speakers={speakers} />
      ),
    }),
    [scan, speakers, t],
  );
}
