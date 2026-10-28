import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  ChevronRight,
  Replace,
  Trash2,
  UserCog,
  Users,
} from "lucide-react";
import { commands, type ProviderSpeaker } from "@/bindings";
import { Input } from "@/components/ui/Input";
import { useSettings } from "@/hooks/useSettings";
import { VOICE_PALETTE, voiceColor } from "@/lib/voices/palette";
import {
  markerAt,
  markersNamed,
  renameSpeakerMarkers,
  resolveSpeaker,
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
import { providerIdOfEngine } from "../useEngineCaps";
import { PROVIDER_VOICES_CHANGED } from "../providers/VisibleVoices";

/**
 * Alle aufgenommenen Fish-Stimmen als Sprecher — Anzeigename und Farbe kommen aus der
 * `meta.json` je Stimme (Registry), nicht aus der id. Die Verwaltung meldet
 * Änderungen über dasselbe Fensterereignis wie die Stimmenliste, damit
 * Chips und Verwaltung nie auseinanderlaufen.
 *
 * Immer die Fish-Stimmen, auch wenn ein Anbieter vorliest: die Stimmenwahl braucht
 * ihre Anzeigenamen. Welche Sprecher im Skript gelten, sagt [`useSpeakers`].
 */
export function useFishSpeakers(): SpeakerRef[] {
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
              language: info.meta.language ?? "",
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

/** Farbe eines Anbieter-Sprechers: stabil je Kennung (Hash in die Palette), wie die
 *  Standardfarbe der Fish-Stimmen. */
export function providerSpeakerColor(id: string): string {
  let hash = 0;
  for (const ch of id) hash = (hash * 31 + (ch.codePointAt(0) ?? 0)) >>> 0;
  const keys = Object.keys(VOICE_PALETTE);
  return voiceColor(keys[hash % keys.length]);
}

/** Die Sprecher eines Anbieters (`tts_provider_speakers`) als `SpeakerRef`. */
export function providerSpeakerRefs(list: ProviderSpeaker[]): SpeakerRef[] {
  return list.map((speaker) => ({
    id: speaker.id,
    displayName: speaker.name,
    aliases: speaker.aliases,
    gender: speaker.gender,
    language: speaker.language,
    color: providerSpeakerColor(speaker.id),
  }));
}

/**
 * Die Sprecher des Anbieters, der gerade vorliest -- genau die Liste, nach der das
 * Backend Marker im Text schaltet (`tts_provider_speakers`). Leer ohne Anbieter und bei
 * einem Anbieter ohne Stimmwechsel.
 *
 * Neu geladen, wenn sich Anbieter, gewaehlte Stimme oder die Auswahl "im Vorlesen
 * anzeigen" aendern (dann sind auch die Sprecher andere) oder die Stimmenverwaltung
 * etwas meldet.
 */
export function useProviderSpeakers(providerId: string | null): SpeakerRef[] {
  const { getSetting } = useSettings();
  const [speakers, setSpeakers] = useState<SpeakerRef[]>([]);
  const chosenVoice = providerId
    ? (getSetting("tts_providers")?.voices?.[providerId] ?? "")
    : "";
  useEffect(() => {
    if (!providerId) {
      setSpeakers([]);
      return;
    }
    let alive = true;
    const load = () => {
      void commands
        .ttsProviderSpeakers()
        .then((result) => {
          if (!alive) return;
          setSpeakers(
            result.status === "ok" && Array.isArray(result.data)
              ? providerSpeakerRefs(result.data)
              : [],
          );
        })
        .catch(() => alive && setSpeakers([]));
    };
    load();
    window.addEventListener("lv-voices-changed", load);
    window.addEventListener(PROVIDER_VOICES_CHANGED, load);
    return () => {
      alive = false;
      window.removeEventListener("lv-voices-changed", load);
      window.removeEventListener(PROVIDER_VOICES_CHANGED, load);
    };
  }, [providerId, chosenVoice]);
  return speakers;
}

/**
 * Die Sprecher, die im Skript gelten: bei Fish die aufgenommenen Stimmen, bei einem
 * Anbieter mit Stimmwechsel (Phonon u. a.) seine sichtbaren Stimmen. Eine Quelle fuer
 * Editor (Chips, "Sprecher einfuegen"), Skript-Pruefung und Sprecher-Tausch.
 */
export function useSpeakers(): SpeakerRef[] {
  const { getSetting } = useSettings();
  const providerId = providerIdOfEngine(getSetting("tts_engine") ?? "fish");
  const fish = useFishSpeakers();
  const provider = useProviderSpeakers(providerId);
  return providerId ? provider : fish;
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

/** "weiblich"/"männlich" für das Geschlecht, das ein Anbieter meldet (wie in der
 *  Stimmenverwaltung der Anbieter); `null` ohne Angabe. */
export function useGenderLabel(): (
  gender: string | null | undefined,
) => string | null {
  const { t } = useTranslation();
  return useCallback(
    (gender) =>
      gender
        ? t(`tts.providers.voices.gender.${gender}`, { defaultValue: gender })
        : null,
    [t],
  );
}

/** Ein Sprecher in einer Liste: Farbpunkt + Name (+ Geschlecht bei Anbieter-Stimmen),
 *  44 px hoch (Touch). */
const SpeakerRow: React.FC<{
  speaker: SpeakerRef;
  current?: boolean;
  onPick: (speaker: SpeakerRef) => void;
}> = ({ speaker, current = false, onPick }) => {
  const genderLabel = useGenderLabel();
  const gender = genderLabel(speaker.gender);
  return (
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
      {gender && (
        <span
          className="shrink-0 text-xs text-text/45"
          data-testid="speaker-gender"
        >
          {gender}
        </span>
      )}
      <span className="shrink-0 text-xs text-text/45">
        {speakerMarkerText(speaker)}
      </span>
    </button>
  );
};

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

const MENU_ROW = "lv-menu-item";

/**
 * Abschnitt „Sprecher ändern" im Kontextmenü: steht der Rechtsklick auf einem
 * Marker `<Name>` (bekannt oder nicht), lässt er sich auf einen anderen
 * Sprecher umhängen -- nur an dieser Stelle oder an allen Stellen mit diesem
 * Namen. Der Stil jeder Stelle bleibt.
 */
const SpeakerChangeSection: React.FC<{
  api: ChipMenuApi;
  speakers: SpeakerRef[];
}> = ({ api, speakers }) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [picked, setPicked] = useState<SpeakerRef | null>(null);
  const { query, setQuery, filtered, showSearch } = useSpeakerSearch(speakers);
  const text = api.text ?? "";
  const marker =
    api.caret === undefined ? undefined : markerAt(text, api.caret);
  if (!marker || !api.replaceRange || !api.replaceText || speakers.length === 0)
    return null;
  const current = resolveSpeaker(marker.name, speakers);
  const count = markersNamed(text, marker.name).length;
  const choices = filtered.filter((s) => s.id !== current?.id);
  const replaceEverywhere = api.replaceEverywhere;

  const applyHere = (speaker: SpeakerRef) =>
    api.replaceRange?.(
      marker.start,
      marker.end,
      speakerMarkerText(speaker, marker.style),
    );
  const applyAll = (speaker: SpeakerRef) =>
    api.replaceText?.(renameSpeakerMarkers(text, marker.name, speaker));

  return (
    <>
      <div className="my-1 border-t border-mid-gray/15" aria-hidden="true" />
      <button
        type="button"
        role="menuitem"
        aria-expanded={open}
        data-testid="menu-speaker-change"
        onClick={() => {
          setOpen((v) => !v);
          setPicked(null);
        }}
        className={MENU_ROW}
      >
        <UserCog width={15} height={15} aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate">
          {t("tts.speakers.menuChange", { name: marker.name })}
        </span>
        {open ? (
          <ChevronDown width={15} height={15} aria-hidden="true" />
        ) : (
          <ChevronRight width={15} height={15} aria-hidden="true" />
        )}
      </button>
      {replaceEverywhere && (
        <button
          type="button"
          role="menuitem"
          data-testid="menu-speaker-replace-everywhere"
          onClick={() => replaceEverywhere(marker.name)}
          className="lv-menu-item"
        >
          <Replace width={15} height={15} aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate">
            {t("tts.replaceSpeaker.menu", { name: marker.name })}
          </span>
        </button>
      )}
      {open && picked && (
        <div
          className="border-t border-mid-gray/15 px-2 py-1"
          data-testid="menu-speaker-change-scope"
        >
          <p className="px-2 pb-1 pt-2 text-xs text-text/60">
            {t("tts.speakers.changeTo", {
              from: marker.name,
              to: picked.displayName,
            })}
          </p>
          <button
            type="button"
            role="menuitem"
            data-testid="menu-speaker-change-here"
            onClick={() => applyHere(picked)}
            className={MENU_ROW}
          >
            {t("tts.speakers.changeHere")}
          </button>
          <button
            type="button"
            role="menuitem"
            data-testid="menu-speaker-change-all"
            onClick={() => applyAll(picked)}
            className={MENU_ROW}
          >
            {t("tts.speakers.changeAll", { count, name: marker.name })}
          </button>
        </div>
      )}
      {open && !picked && (
        <div className="border-t border-mid-gray/15 px-2 py-1">
          {showSearch && (
            <div className="px-1 pb-1">
              <SpeakerSearchField value={query} onChange={setQuery} />
            </div>
          )}
          {choices.length === 0 ? (
            <p className="px-2 py-3 text-xs text-text/50">
              {t("tts.speakers.emptySearch")}
            </p>
          ) : (
            <div className="max-h-64 overflow-y-auto">
              {choices.map((speaker) => (
                <SpeakerRow
                  key={speaker.id}
                  speaker={speaker}
                  // Kommt der Name nur einmal vor, gibt es nichts zu fragen.
                  onPick={(s) => (count > 1 ? setPicked(s) : applyHere(s))}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </>
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
        className="lv-menu-item"
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
        <>
          <SpeakerChangeSection api={api} speakers={speakers} />
          <SpeakerMenuSection api={api} speakers={speakers} />
        </>
      ),
    }),
    [scan, speakers, t],
  );
}
