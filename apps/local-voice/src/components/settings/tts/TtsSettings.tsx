import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import {
  commands,
  type AutoTagOptions,
  type Book,
  type PageInfo,
  type TtsStatus,
} from "@/bindings";
import { useTextHistory } from "../../../hooks/useTextHistory";
import { HistoryButtons } from "./editor/HistoryButtons";
import { defaultAutoTagOptions } from "./tags/AutoTagDialog";
import { canonicalizeTags, canonicalizeTagsAt } from "@/lib/tags/registry";
import { useTagLanguage } from "./tags/tagLanguage";
import { ScriptWorkshopDialog } from "./books/ScriptWorkshopDialog";
import { audioExportName } from "@/lib/utils/exportName";
import { openModelsArea } from "@/components/settings/models/ModelsSettings";
import { openVoiceManager } from "@/components/settings/models/voices";
import { useSettings } from "../../../hooks/useSettings";
import { ShortcutInput } from "../ShortcutInput";
import {
  FilesSidebar,
  ColumnStrip,
  PagesSidebar,
  TtsHelpDock,
} from "./WorkspaceSidebars";
import { ResizeHandle } from "../../ui/ResizeHandle";
import { useWorkspaceLayout } from "./useWorkspaceLayout";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { PageShell } from "../../ui/PageShell";
import { SettingContainer } from "../../ui/SettingContainer";
import { Input } from "../../ui/Input";
import { Button } from "../../ui/Button";
import { IconButton } from "../../ui/IconButton";
import { Dialog } from "../../ui/Dialog";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { Slider } from "../../ui/Slider";
import { Select } from "../../ui/Select";
import { TabList } from "../../ui/TabList";
import {
  TtsChipEditor,
  type ChipEditorFinding,
  type ChipEditorInsertApi,
  type ChipEditorSuggestion,
} from "./editor/TtsChipEditor";
import { ScriptCheckPanel } from "./editor/ScriptCheckPanel";
import {
  checkScript,
  isHintFinding,
  replaceFindingEverywhere,
  type ScriptFinding,
} from "@/lib/voices/scriptCheck";
import { useTagProvider } from "./tags/tagProvider";
import {
  providerSpeakerRefs,
  useSpeakerProvider,
  useSpeakers,
} from "./speakers";
import { useEngineCaps } from "./useEngineCaps";
import { SpeakerSwapDialog } from "./speakers/SpeakerSwapDialog";
import { ReplaceSpeakerDialog } from "./speakers/ReplaceSpeakerDialog";
import type { SentenceClock } from "./reader/useSentenceClock";
import { useContextMenu } from "../../ui/ContextMenu";
import {
  applySwap,
  speakersInText,
  suggestSwap,
  type LangVoice,
} from "@/lib/voices/speakerSwap";
import { voiceColor } from "@/lib/voices/palette";
import { resolveSuggestion } from "./tags/AutoTagBar";
import { ReadAloudActions } from "./ReadAloudActions";
import { TooltipTrigger } from "../../ui/TooltipTrigger";
import { documentTitle, takeLibraryAutoplay } from "@/lib/library/books";
import {
  clearResumePoint,
  loadResumePoint,
  saveResumePoint,
} from "../../../lib/reader/resumePoint";
import {
  addBookmark,
  loadBookmarks,
  removeBookmark,
  type Bookmark,
} from "../../../lib/reader/bookmarks";
import {
  bookInfo,
  buildContents,
  chapterAt,
  type ContentsEntry,
} from "../../../lib/reader/contents";
import { usePersistentState } from "../../../hooks/usePersistentState";
import { registerPageHelp } from "../../help/pageHelp";
import { useDebouncedValue } from "../../../hooks/useDebouncedValue";
import {
  TTS_TARGET_LANGS,
  targetLangCode,
} from "../../../lib/constants/languages";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Glyph } from "../../ui/AudioPlayer";
import {
  HelpCircle,
  Mic,
  PanelRightClose,
  Replace,
  Settings,
  TriangleAlert,
} from "lucide-react";
import { useVoiceChoice } from "./useVoiceChoice";
import { SamplePlayButton } from "./providers/SamplePlayButton";
import { VoiceTryButton } from "./providers/VoiceTryButton";
import { sentenceAfterProgress, shownSentenceNumber } from "./sentenceCounter";
import {
  ReaderView,
  type ReaderMark,
  type ReaderPageInfo,
  type ReaderViewApi,
} from "./reader/ReaderView";
import { ReaderPlayerBar } from "./reader/ReaderPlayerBar";
import { ReaderBookmarks } from "./reader/ReaderBookmarks";
import { ReaderContents } from "./reader/ReaderContents";
import { IsbnDialog } from "./reader/IsbnDialog";
import { applyBookTitle, findIsbn } from "@/lib/reader/isbn";
import type { IsbnBook } from "@/bindings";
import {
  READER_FONTS,
  READER_THEMES,
  ReaderToolbar,
} from "./reader/ReaderToolbar";
import type { ReaderFont, ReaderTheme } from "./reader/ReaderToolbar";
import { InfoPopover } from "./reader/InfoPopover";
import { useKeepAwake } from "./reader/useKeepAwake";
import { parseReaderDoc } from "@/lib/reader/readerDoc";
import { stripPageMarks, toSpokenScript } from "@/lib/reader/spokenScript";
import {
  SentenceLocator,
  type SourceRange,
} from "@/lib/reader/sentenceLocator";

/// Abspieltempo der Transportleiste. Bewusst grob gestuft: feiner regelt der
/// Schieber in den Einstellungen, hier will man im Hoeren einmal schneller
/// oder langsamer stellen, nicht justieren.
/// Tempostufen der Transportleiste. 0,5x ist dabei, seit der feinstufige
/// Regler aus den Einstellungen entfallen ist: dieselbe Einstellung an zwei
/// Orten war eine Dublette, aber der langsamste Wert soll bleiben.
const SPEEDS = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
const speedLabel = (value: number) =>
  `${value.toFixed(2).replace(".", ",")}\u00d7`;

/// Erlaubte MP3-Bitraten (kbit/s) — dieselben vier Stufen wie in
/// `settings.rs`; mehr Stufen muesste die Oberflaeche auch erklaeren.
const EXPORT_BITRATES = [128, 192, 256, 320];

/// Glaettung der Restzeit-Schaetzung: der neu gemessene Wert geht mit 30 %
/// ein. Die ersten Saetze sind langsamer (das Modell laedt noch), eine rohe
/// Hochrechnung springt deshalb von Satz zu Satz wild.
const ETA_SMOOTHING = 0.3;
/** Buchteile (lva#150): so viele Sätze vor dem Ende beginnt der Anlauf des
 *  nächsten Teils — so viele, wie der Vorpuffer vorausrechnet. */
const PART_LEAD_SENTENCES = 3;

/// Restdauer als "2:30", ab einer Stunde als "1:05:00". Auf 5 Sekunden
/// gerundet: sekundengenau zappelt die Zahl bei jedem Satz, ohne mehr zu
/// sagen als die gerundete.
const formatEta = (seconds: number) => {
  const total = Math.max(5, Math.round(seconds / 5) * 5);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = String(total % 60).padStart(2, "0");
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${secs}`;
  }
  return `${minutes}:${secs}`;
};

/** Fehlercode des Backends, wenn der Nutzer abgebrochen hat. */
const TRANSFORM_CANCELLED = "transform_cancelled";

/** Verlauf der Uebersetzung: je Seite UND Sprache (Englisch und Franzoesisch
 *  haben jeweils ihre eigenen Schritte). */
const translationHistoryKey = (page: string | null | undefined, lang: string) =>
  `${page ?? "-"}:translation:${lang}`;

export const TtsSettings = () => {
  const { t, i18n } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const uiLang = i18n.language?.split("-")[0] ?? "en";
  const tagLang = useTagLanguage();
  /** Tag- und Sprecher-Chips in allen drei Text-Reitern. Reihenfolge zählt
   *  nur bei gleichem Startoffset — Tags (`[…]`) und Sprecher (`<…>`,
   *  `Name:`) können sich nicht überschneiden. */
  const tagProvider = useTagProvider();
  const speakerProvider = useSpeakerProvider();
  const chipProviders = useMemo(
    () => [tagProvider, speakerProvider],
    [tagProvider, speakerProvider],
  );
  /** Bekannte Sprecher fuer die Skript-Pruefung -- dieselbe Liste, gegen die
   *  auch die Sprecher-Chips abgeglichen werden: bei Fish die aufgenommenen
   *  Stimmen, bei einem Anbieter mit Stimmwechsel (Phonon u. a.) seine Stimmen. */
  const speakers = useSpeakers();
  /** Ansicht beim Öffnen eines Texts wählen (gesetzt weiter unten, wo die Wahl lebt). */
  const openModeRef = useRef<((loaded: string) => void) | null>(null);
  /** Was die gewaehlte Engine kann (Sprecher im Skript, Stil-Tags): die Seite
   *  richtet sich nach den Faehigkeiten, nicht nach dem Namen der Engine. */
  const engineCaps = useEngineCaps();
  /** Ein Anbieter ohne Stil-Tags (Phonon): nur Pausen anbieten und erklaeren. Fish und
   *  Piper behalten ihre bisherige Darstellung. */
  const pausesOnly = engineCaps.kind === "provider" && !engineCaps.styleTags;
  /** Skript-Pruefung: Bestaetigungsdialog vor Vorlesen/Speichern, wenn es
   *  Befunde gibt. Traegt, was nach "Trotzdem" laufen soll. */
  const [checkDialog, setCheckDialog] = useState<"speak" | "save" | null>(null);
  /** Einfüge-API des Editors im AKTIVEN Reiter (es ist immer nur einer
   *  gemountet) — Ziel für Palette-Klick und Palette-Drag. */
  const editorApiRef = useRef<ChipEditorInsertApi | null>(null);
  const [status, setStatus] = useState<TtsStatus | null>(null);
  // The text you were about to have read out survives leaving the page —
  // losing a pasted article because you glanced at the model list is the
  // kind of loss nobody forgives.
  // Der Arbeitsstand gehoert der SEITE, nicht mehr der App: Text,
  // Zusammenfassung, Quelle und offener Reiter werden je Seite geladen und
  // gespeichert (state.json im Seitenordner). Die localStorage-Werte von
  // frueher werden einmalig in die erste Seite uebernommen.
  const [text, setText] = useState<string>("");
  // Auto-Tagging-Einstellungen der Seite (state.json); ohne eigene die
  // zuletzt benutzten aus den Einstellungen, sonst die Voreinstellung.
  const [autoTagOptions, setAutoTagOptions] = useState<AutoTagOptions | null>(
    null,
  );
  /** T4 Auto-Tagging: offene Vorschläge im Original-Reiter (gestrichelte
   *  Chips im Editor). Gehört der Seite hier, weil sowohl der Editor
   *  (Popover-Buttons) als auch AutoTagBar ("Alle annehmen/verwerfen")
   *  dieselbe Liste + denselben Text splicen müssen. */
  const [tagSuggestions, setTagSuggestions] = useState<ChipEditorSuggestion[]>(
    [],
  );
  /** Text, gegen den `tagSuggestions`-Offsets aktuell gültig sind (Review-
   *  Befund 2: Race zwischen Erzeugung/Anzeige eines Vorschlags und seiner
   *  Anwendung). `null` ohne offene Liste. Wird bei JEDER erfolgreichen
   *  Text-Änderung (Einzel- oder Alle-annehmen) auf den neuen Text
   *  nachgezogen — weicht der tatsächliche `text` trotzdem davon ab, hat der
   *  Nutzer zwischendurch getippt, und Annehmen verwirft statt zu splicen. */
  const [tagSuggestionsSourceText, setTagSuggestionsSourceText] = useState<
    string | null
  >(null);
  const [pages, setPages] = useState<PageInfo[]>([]);
  const [activePage, setActivePage] = usePersistentState<string>(
    "tts.activePage",
    "",
  );
  // Beide Seitenspalten starten offen. Eingeklappt zeigten sie nur ein
  // 18-Pixel-Symbol, und wer sie nie aufklappte, hielt sie fuer Reiter ohne
  // Funktion — obwohl darin die Arbeitsblaetter und die erzeugten Aufnahmen
  // liegen. Wer sie zuklappt, behaelt das (persistent).
  // ".v3"-Schluessel: die vorherigen Schluessel haben bei Bestandsnutzern den
  // alten Wert persistiert — ohne Umbenennung saehe niemand den neuen Default.
  const [pagesCollapsed, setPagesCollapsed] = usePersistentState<string>(
    "tts.pagesCollapsed.v3",
    "0",
  );
  const [filesCollapsed, setFilesCollapsed] = usePersistentState<string>(
    "tts.filesCollapsed.v3",
    "0",
  );
  // Rechte Leiste: Dateien oder Hilfe. Die Hilfe ist ein Reiter derselben
  // Leiste, kein Fenster — sie soll neben dem Text stehen, nicht davor.
  // Erster Start: Hilfe, damit niemand die Leiste fuer eine leere
  // Dateiliste haelt. Ein gespeicherter Reiter hat immer Vorrang.
  const [helpOpenValue, setHelpOpenValue] = usePersistentState<string>(
    "tts.helpOpen",
    "0",
  );
  const helpOpen = helpOpenValue === "1";
  // Das Hilfe-Menü der Titelleiste schaltet dieselbe Hilfe.
  useEffect(
    () =>
      registerPageHelp(() =>
        setHelpOpenValue(helpOpenValue === "1" ? "0" : "1"),
      ),
    [helpOpenValue, setHelpOpenValue],
  );
  // Anordnung rechts vom Editor (gestapelt oder nebeneinander) und die
  // per Ziehgriff verstellbaren Breiten; alles bleibt ueber Neustarts.
  // Rechte Spalte ganz ausblenden (Variante A, Codex 01.11.).
  const [rightHiddenStored, setRightHidden] = usePersistentState<"1" | "0">(
    "tts.rightHidden",
    "0",
  );
  const rightHidden = rightHiddenStored === "1";
  const ws = useWorkspaceLayout(
    pagesCollapsed === "1",
    filesCollapsed === "1",
    rightHidden,
  );
  /** Erst nach dem Laden einer Seite darf gespeichert werden — sonst
   *  ueberschriebe der leere Anfangszustand den echten. */
  const pageLoaded = useRef(false);
  /** Zählt fertig geladene Seiten (Buchteile: Weiterlesen nach dem Laden). */
  const [pageLoadTick, setPageLoadTick] = useState(0);
  /** Zu welchem Reiter die laufende Sprechsitzung gehoert. Fortsetzen und
   *  Satzspruenge gelten nur fuer sie — ein anderer Reiter startet neu. */
  const sessionTab = useRef<"original" | "translation" | "summary">("original");
  /** Zielsprache der Uebersetzung — dieselbe Einstellung wie in der
   *  Audio-Uebersetzung, damit man sie nicht an zwei Stellen pflegt. */
  const targetLang = getSetting("tts_translate_lang") ?? "English";
  /** Dialog: die aktuelle Seed-Stimme unter einem Namen sichern. */
  /** Zusammenfassung des Originals — dritter Reiter. */
  const [summary, setSummary] = useState<string>("");
  const [summarizing, setSummarizing] = useState(false);
  const [tidying, setTidying] = useState(false);
  const [sumLength, setSumLength] = usePersistentState<string>(
    "tts.summary.length",
    "mittel",
  );
  const [sumDetail, setSumDetail] = usePersistentState<string>(
    "tts.summary.detail",
    "ausgewogen",
  );
  const [sumAudience, setSumAudience] = usePersistentState<string>(
    "tts.summary.audience",
    "allgemein",
  );
  const [sourceUrl, setSourceUrl] = useState<string>("");
  const [loadingSource, setLoadingSource] = useState(false);
  const [urlDialogOpen, setUrlDialogOpen] = useState(false);
  const [workshopOpen, setWorkshopOpen] = useState(false);
  /** Welcher Reiter offen ist. Das Original bleibt immer erhalten. */
  const [tab, setTab] = useState<"original" | "translation" | "summary">(
    "original",
  );
  // Stimme je Seite UND Reiter, gespeichert mit der Seite (state.json): das
  // Original liest Thorsten, die Uebersetzung Lessac -- und jede Seite behaelt
  // ihre eigene Wahl. Wer die Seite wechselt, bekommt deren Stimme (Patrick
  // 28.10.2026). Werte wie im Dropdown: "@default", "<fish-id>", "piper:<id>",
  // "provider:<id>:<stimme>". Fehlt ein Eintrag, bleibt die aktuelle Stimme.
  const [tabVoices, setTabVoices] = useState<Record<string, string>>({});
  /** "Sprecher auf allen Seiten ersetzen": offen, mit optionaler Vorbelegung. */
  const [replaceSpeaker, setReplaceSpeaker] = useState<{
    from?: string;
  } | null>(null);
  const voiceMenu = useContextMenu(t("tts.voices.title"));
  const [translation, setTranslation] = useState<string | null>(null);
  /** Sprachen, in die diese Seite uebersetzt ist (Reiter ueber dem Feld). */
  const [translatedLangs, setTranslatedLangs] = useState<string[]>([]);
  const addTranslatedLang = (lang: string) =>
    setTranslatedLangs((cur) => (cur.includes(lang) ? cur : [...cur, lang]));
  /** Zu welchem Text/welcher Sprache `translation` gehoert, und was davon schon im Speicher liegt. */
  const translationFor = useRef("");
  const savedTranslation = useRef<string | null>(null);
  const translationKey = (t: string, lang: string) => `${t}\u0000${lang}`;
  const [translating, setTranslating] = useState(false);
  const [dictating, setDictating] = useState(false);
  const [startingSeconds, setStartingSeconds] = useState(0);
  const [lastError, setLastError] = useState<string | null>(null);
  /** Fortschritt von Übersetzung/Zusammenfassung (Backend: Block für Block). */
  const [transformProgress, setTransformProgress] = useState<{
    kind: "translation" | "summary";
    done: number;
    total: number;
  } | null>(null);
  useEffect(() => {
    const un = listen<{
      kind: "translation" | "summary";
      done: number;
      total: number;
    }>("tts-transform-progress", (e) => setTransformProgress(e.payload));
    return () => void un.then((off) => off());
  }, []);
  /** Piper mit Spracherkennung: die Stimme, die den Text des Reiters WIRKLICH liest. */
  const [piperReader, setPiperReader] = useState<string | null>(null);
  /** Sprecher tauschen: Stimmen mit Sprache, Fenster, Vorbelegung. */
  const [swapOpen, setSwapOpen] = useState(false);
  const [langVoices, setLangVoices] = useState<LangVoice[]>([]);
  const [swapInitial, setSwapInitial] = useState<Record<string, string>>({});
  /** Der Vorlesetext wurde an der Zeichengrenze gekappt. */
  const [truncated, setTruncated] = useState<{
    limit: number;
    total: number;
  } | null>(null);
  const [speakProgress, setSpeakProgress] = useState<{
    position: number;
    total: number;
  } | null>(null);
  /** Fortschritt ohne Render-Verzug — der Listener setzt ihn sofort. */
  const speakProgressRef = useRef(speakProgress);
  speakProgressRef.current = speakProgress;
  const [currentSentence, setCurrentSentence] = useState<string | null>(null);
  /** Nullbasierter Index des Satzes, der gerade zu hoeren ist (fuer den Zaehler "Satz n/m"). */
  const [currentIndex, setCurrentIndex] = useState<number | null>(null);
  /** Start und Dauer des hörbaren Satzes: Zeit und Umblättern laufen damit
   *  sekundengenau mit, nicht nur Satz für Satz. */
  const [sentenceStart, setSentenceStart] = useState<{
    index: number | null;
    at: number;
    ms: number;
  } | null>(null);
  const [saving, setSaving] = useState(false);
  const [exportProgress, setExportProgress] = useState<{
    position: number;
    total: number;
  } | null>(null);
  /** Geschaetzte Restdauer des Exports in Sekunden; null = noch zu frueh. */
  const [exportEta, setExportEta] = useState<number | null>(null);
  /** Letzter Messpunkt (Zeit + Satzposition) der Restzeit-Schaetzung. */
  const exportTick = useRef<{ time: number; position: number } | null>(null);
  /** Geglaetteter Mittelwert der Zeit je Satz in Millisekunden. */
  const exportPerSentence = useRef<number | null>(null);
  /** Der Pfad, den der Lauf TATSAECHLICH schreibt (Endung nach Format). */
  const exportPath = useRef<string | null>(null);
  const startingTimer = useRef<number | null>(null);

  useEffect(() => {
    commands.ttsServerStatus().then((r) => {
      if (r.status === "ok") setStatus(r.data);
    });
    const un = listen<TtsStatus>("tts-state-changed", (e) => {
      setStatus(e.payload);
      if (e.payload.phase === "error" && e.payload.message) {
        setLastError(e.payload.message);
      } else if (
        e.payload.phase === "ready" ||
        e.payload.phase === "speaking"
      ) {
        setLastError(null);
      }
    });
    const unExport = listen<{
      position: number;
      total: number;
      cancelled: boolean;
    }>("tts-export-progress", (e) => {
      const { position, total, cancelled } = e.payload;
      if (cancelled || (total > 0 && position >= total)) {
        setSaving(false);
        setExportProgress(null);
        setExportEta(null);
        exportTick.current = null;
        exportPerSentence.current = null;
        // Fertige Datei: die Dateileiste rechts soll sie sofort zeigen.
        window.dispatchEvent(new CustomEvent("lv-files-changed"));
        // Der Pfad kommt vom Befehl, nicht aus dem Dialog: bei MP3 hat der
        // Lauf die Endung korrigiert, und die Meldung soll die Datei nennen,
        // die es wirklich gibt.
        if (!cancelled && exportPath.current) {
          toast.success(t("tts.exportSaved", { path: exportPath.current }));
        }
        exportPath.current = null;
        return;
      }
      setExportProgress({ position, total });
      // Restzeit: Zeit je Satz messen, glaetten, hochrechnen. Der erste Satz
      // traegt die Ladezeit des Modells — deshalb wird erst ab dem zweiten
      // Satz ueberhaupt etwas angezeigt.
      const now = performance.now();
      const last = exportTick.current;
      if (last && position > last.position) {
        const perSentence = (now - last.time) / (position - last.position);
        exportPerSentence.current =
          exportPerSentence.current === null
            ? perSentence
            : exportPerSentence.current * (1 - ETA_SMOOTHING) +
              perSentence * ETA_SMOOTHING;
      }
      exportTick.current = { time: now, position };
      const remaining = total - position;
      setExportEta(
        position >= 2 && exportPerSentence.current !== null && remaining > 0
          ? (exportPerSentence.current * remaining) / 1000
          : null,
      );
    });
    const unExportError = listen<{ message: string }>(
      "tts-export-error",
      (e) => {
        setSaving(false);
        setExportProgress(null);
        setExportEta(null);
        exportTick.current = null;
        exportPerSentence.current = null;
        exportPath.current = null;
        setLastError(e.payload.message);
      },
    );
    const unProgress = listen<{ position: number; total: number }>(
      "tts-speak-progress",
      (e) => {
        // Sofort, nicht erst beim Rendern: das Ende der Sitzung kann vor dem
        // Render des letzten Fortschritts ankommen (Buchteile, lva#150).
        speakProgressRef.current = e.payload;
        setSpeakProgress(e.payload);
        // Nach einem Sprung oder einem fertigen Satz gilt der alte Satz nicht mehr.
        setCurrentIndex((index) =>
          index === null
            ? null
            : (sentenceAfterProgress({ index }, e.payload.position)?.index ??
              null),
        );
      },
    );
    const unSentence = listen<{
      context: string;
      index?: number;
      text: string;
      duration_ms?: number;
    }>("tts-current-sentence", (e) => {
      if (e.payload.context !== "speak") return;
      setSentenceStart({
        index: typeof e.payload.index === "number" ? e.payload.index : null,
        at: performance.now(),
        ms: e.payload.duration_ms ?? 0,
      });
      setCurrentSentence(e.payload.text);
      setCurrentIndex(
        typeof e.payload.index === "number" ? e.payload.index : null,
      );
    });
    // Eine Kuerzung darf nicht stumm bleiben: sonst hoert das Vorlesen
    // mitten im Text auf, ohne dass irgendwo steht, warum.
    const unTruncated = listen<{ limit: number; total: number }>(
      "tts-text-truncated",
      (e) => setTruncated(e.payload),
    );
    return () => {
      un.then((f) => f());
      unExport.then((f) => f());
      unExportError.then((f) => f());
      unProgress.then((f) => f());
      unSentence.then((f) => f());
      unTruncated.then((f) => f());
    };
  }, []);

  // Beim Wechsel der Sprache (oder des Originaltexts) zeigen, was schon da
  // ist — ohne eine neue Uebersetzung anzustossen. Genau dafuer liegt der
  // Zwischenspeicher auf Platte: hin und her schalten kostet nichts.
  useEffect(() => {
    let abandoned = false;
    if (!text.trim()) {
      setTranslation(null);
      return;
    }
    void commands.ttsCachedTranslation(text, targetLang).then((hit) => {
      if (abandoned) return;
      translationFor.current = translationKey(text, targetLang);
      savedTranslation.current = hit;
      // Geladen, nicht bearbeitet: "Rueckgaengig" darf das nicht leeren.
      historyRef.current?.syncPresent(
        translationHistoryKey(activePageRef.current, targetLang),
        hit ?? "",
      );
      if (hit) addTranslatedLang(targetLang);
      setTranslation(hit);
    });
    return () => {
      abandoned = true;
    };
  }, [text, targetLang]);

  // Seitenliste laden; ohne gueltige aktive Seite wird die erste offen.
  const reloadPages = useCallback(async () => {
    const result = await commands.pagesList();
    if (result.status !== "ok") return;
    setPages(result.data);
    if (!result.data.some((p) => p.id === activePage) && result.data.length) {
      setActivePage(result.data[0].id);
    }
  }, [activePage, setActivePage]);

  useEffect(() => {
    void reloadPages();
  }, [reloadPages]);

  /** Neue Seite mit Text anlegen und aktiv machen (Skript-Werkstatt). */
  const createPageWithText = useCallback(
    async (title: string, text: string): Promise<string | null> => {
      const created = await commands.pagesCreate(title);
      if (created.status !== "ok") return null;
      await commands.pageStateSave(
        created.data.id,
        JSON.stringify({ text, summary: "", sourceUrl: "", tab: "original" }),
      );
      await reloadPages();
      setActivePage(created.data.id);
      return created.data.id;
    },
    [reloadPages, setActivePage],
  );

  // Arbeitsstand der aktiven Seite laden. Die erste Seite uebernimmt
  // einmalig, was frueher app-weit im localStorage lag — sonst waere der
  // Text, der beim Update im Feld stand, kommentarlos weg.
  useEffect(() => {
    if (!activePage) return;
    pageLoaded.current = false;
    void commands.pageStateLoad(activePage).then((result) => {
      if (result.status !== "ok") return;
      if (result.data) {
        try {
          const state = JSON.parse(result.data) as {
            text?: string;
            summary?: string;
            sourceUrl?: string;
            tab?: string;
            voices?: Record<string, string>;
            translatedLangs?: string[];
            autoTag?: AutoTagOptions;
          };
          setTranslatedLangs(
            Array.isArray(state.translatedLangs) ? state.translatedLangs : [],
          );
          setText(state.text ?? "");
          openModeRef.current?.(
            state.tab === "summary" ? (state.summary ?? "") : (state.text ?? ""),
          );
          setAutoTagOptions(state.autoTag ?? null);
          setSummary(state.summary ?? "");
          setSourceUrl(state.sourceUrl ?? "");
          setTabVoices(
            state.voices && typeof state.voices === "object"
              ? state.voices
              : {},
          );
          setTab(
            state.tab === "translation" || state.tab === "summary"
              ? state.tab
              : "original",
          );
        } catch {
          setText(result.data);
          openModeRef.current?.(result.data);
        }
      } else {
        // Einmalige Uebernahme der Werte aus der Zeit vor den Seiten — und
        // zwar wirklich EINmalig: die Schluessel werden nach dem Lesen
        // geloescht. Ohne das erbte jede neue, leere Seite denselben alten
        // Text, und "jede Seite hat ihren eigenen Inhalt" waere gelogen.
        const legacy = (key: string) => {
          const storageKey = `lva.ui.${key}`;
          const value = window.localStorage.getItem(storageKey) ?? "";
          window.localStorage.removeItem(storageKey);
          return value;
        };
        const legacyText = legacy("tts.text");
        setText(legacyText);
        openModeRef.current?.(legacyText);
        setSummary(legacy("tts.summary"));
        setSourceUrl(legacy("tts.summary.url"));
        setTabVoices({});
        setTranslatedLangs([]);
        setTab("original");
      }
      setTranslation(null);
      pageLoaded.current = true;
      setPageLoadTick((n) => n + 1);
    });
  }, [activePage]);

  // Ein anderes Geraet hat Seiten geschickt: Liste neu laden.
  useEffect(() => {
    const unlisten = listen("sync-changed", () => {
      void reloadPages();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [reloadPages]);

  // Arbeitsstand sichern — gebuendelt, eine halbe Sekunde nach der letzten
  // Aenderung. Jeder Tastendruck einzeln waere ein Schreibzugriff zu viel.
  useEffect(() => {
    if (!activePage || !pageLoaded.current) return;
    const handle = window.setTimeout(() => {
      void commands
        .pageStateSave(
          activePage,
          JSON.stringify({
            text,
            summary,
            sourceUrl,
            tab,
            voices: tabVoices,
            translatedLangs,
            autoTag: autoTagOptions ?? undefined,
          }),
        )
        // Die Seitenliste zeigt Vorschau und Zeitpunkt — die stammen aus
        // genau dieser Datei und sollen nicht erst beim naechsten Start
        // stimmen. Und der Geraete-Sync soll die Aenderung zeitnah sehen.
        .then(() => {
          void reloadPages();
          void commands.syncTouch();
        });
    }, 500);
    return () => window.clearTimeout(handle);
  }, [
    activePage,
    text,
    summary,
    sourceUrl,
    tab,
    tabVoices,
    translatedLangs,
    autoTagOptions,
  ]);

  // Sekundenzähler nur während des Serverstarts.
  useEffect(() => {
    if (status?.phase === "starting") {
      if (startingTimer.current === null) {
        setStartingSeconds(0);
        startingTimer.current = window.setInterval(
          () => setStartingSeconds((s) => s + 1),
          1000,
        );
      }
    } else if (startingTimer.current !== null) {
      window.clearInterval(startingTimer.current);
      startingTimer.current = null;
      setStartingSeconds(0);
    }
    return () => {
      if (startingTimer.current !== null) {
        window.clearInterval(startingTimer.current);
        startingTimer.current = null;
      }
    };
  }, [status?.phase]);

  const phase = status?.phase ?? "stopped";
  const speaking = phase === "speaking";
  const playbackSpeed = getSetting("tts_speed") ?? 1.0;
  const sentenceClock = useMemo<SentenceClock | null>(
    () =>
      speaking && sentenceStart && sentenceStart.ms > 0
        ? { ...sentenceStart, speed: playbackSpeed }
        : null,
    [speaking, sentenceStart, playbackSpeed],
  );
  const starting = phase === "starting";

  const speak = async () => {
    // Fortgesetzt wird nur die Sitzung DESSELBEN Reiters. Ohne diese
    // Pruefung galt nach einer Pause im Original "Fortsetzen moeglich" —
    // und Play auf dem Uebersetzungs-Reiter setzte die alte Sitzung fort,
    // las also das Original, obwohl die Uebersetzung offen war.
    if (canResume && sessionTab.current === tab) {
      await resumeSpeaking();
      return;
    }
    // Skript-Pruefung: ein Befund blockiert nicht, fragt aber nach -- nur
    // beim Start, nicht beim Fortsetzen derselben Sitzung. Frisch geprueft,
    // nicht aus der verzoegerten Liste: wer direkt nach dem Tippen auf Play
    // drueckt, soll nicht durch das Debounce-Fenster rutschen.
    if (freshFindings().length > 0) {
      setCheckDialog("speak");
      return;
    }
    await speakNow();
  };

  /** Ab dem Satz an der Rechtsklick-Stelle vorlesen -- oder nur diesen. */
  const speakFrom = async (charOffset: number, onlyOne: boolean) => {
    setLastError(null);
    setSpeakProgress(null);
    setCurrentIndex(null);
    setTruncated(null);
    sessionTab.current = tab;
    // Die Engine bekommt den Text ohne Markdown (lva#148); der Offset wandert
    // mit — ebenso beim Umschreiben der Tags in die englische Form.
    const script = toSpokenScript(spokenText);
    // Offset über eine Marke im ganzen Text abbilden statt über den
    // abgeschnittenen Anfang: ein Schnitt in *kursiv* oder einer Überschrift
    // verschöbe sonst die Stelle um ein Zeichen.
    const MARK = "\uE000";
    const scriptOffset =
      script === spokenText
        ? charOffset
        : Math.max(
            0,
            toSpokenScript(
              spokenText.slice(0, charOffset) + MARK + spokenText.slice(charOffset),
            ).indexOf(MARK),
          );
    locatorRef.current?.reset(charOffset);
    lastLocated.current = null;
    // „Nur diesen Satz“ verschiebt die gemerkte Stelle nicht.
    sessionResumeKey.current = onlyOne ? null : resumeKeyRef.current;
    const canon = canonicalizeTagsAt(script, scriptOffset);
    // „Nur diesen Satz“ liest nie im nächsten Teil weiter.
    if (onlyOne) partSession.current = null;
    const session = onlyOne ? null : startPartSession();
    const result = await commands.ttsSpeakTextFrom(
      canon.text,
      canon.offset,
      onlyOne,
    );
    if (result.status === "error") {
      endPartSession(session);
      setLastError(result.error);
    } else continueWithNextPart(session);
  };

  // Vorab-Erzeugung: geaenderte Saetze in den Cache, ohne Wiedergabe.
  // Laeuft auch waehrend des Vorlesens; danach kann der Server weg.
  const [prewarm, setPrewarm] = useState<{
    done: number;
    total: number;
  } | null>(null);
  useEffect(() => {
    const un = listen<{ done: number; total: number; cancelled: boolean }>(
      "tts-prewarm-progress",
      (e) => {
        const { done, total, cancelled } = e.payload;
        if (cancelled || done >= total) {
          setPrewarm(null);
          if (!cancelled && total > 0) {
            toast.success(t("tts.prewarm.done", { count: total }));
          } else if (!cancelled) {
            toast.info(t("tts.prewarm.nothing"));
          }
          return;
        }
        setPrewarm({ done, total });
      },
    );
    return () => {
      un.then((f) => f());
    };
  }, [t]);
  const startPrewarm = async () => {
    setLastError(null);
    setPrewarm({ done: 0, total: 0 });
    const result = await commands.ttsPrewarm(
      canonicalizeTags(toSpokenScript(spokenText)),
    );
    if (result.status === "error") {
      setPrewarm(null);
      setLastError(result.error);
    }
  };

  const speakNow = async () => {
    // Weiterlesen ab der gemerkten Stelle dieser Seite (lva#148).
    const resumeOffset = loadResumePoint(resumeKeyRef.current, spokenText);
    if (resumeOffset !== null) {
      setFollow(true);
      await speakFrom(resumeOffset, false);
      return;
    }
    setLastError(null);
    setFollow(true);
    setSpeakProgress(null);
    setCurrentIndex(null);
    setTruncated(null);
    sessionTab.current = tab;
    locatorRef.current?.reset(0);
    lastLocated.current = null;
    sessionResumeKey.current = resumeKeyRef.current;
    const session = startPartSession();
    const result = await commands.ttsSpeakText(
      canonicalizeTags(toSpokenScript(spokenText)),
    );
    if (result.status === "error") {
      endPartSession(session);
      setLastError(result.error);
    } else continueWithNextPart(session);
  };

  /**
   * Stops playback outright — this is a cancel, not a suspend; "Fortsetzen"
   * restarts from the last fully spoken sentence.
   *
   * Deliberately NEVER disabled. It used to be gated on `speaking`, which is
   * derived from a phase event — and any event that put the phase back to
   * "ready" mid-playback (a server health check did exactly that) left the
   * only stop control greyed out while audio kept running. Cancelling when
   * nothing is playing costs nothing; being unable to cancel costs the user
   * their loudspeakers.
   */
  /** Pause: anhalten, Position behalten — Play setzt genau dort fort. */
  const pauseSpeaking = () => {
    void commands.ttsCancel();
  };

  /**
   * Stop: anhalten UND an den Anfang. Das ist der Unterschied zu Pause und der
   * einzige Grund, warum das Design-System beide nebeneinander erlaubt —
   * decken sie sich, gehoert Stopp weg.
   */
  const stopSpeaking = () => {
    void commands.ttsCancel();
    partSession.current = null;
    lastSessionPage.current = null;
    autoplayPage.current = null;
    setSpeakProgress(null);
    setCurrentIndex(null);
    // Stopp gilt der Sitzung, nicht der gerade offenen Seite.
    forgetResumePoint(sessionResumeKey.current ?? resumeKeyRef.current);
  };

  const resumeSpeaking = async () => {
    setLastError(null);
    // Nach einer Pause setzt Play die Sitzung derselben Seite fort — dann
    // gilt auch das Weiterlesen im nächsten Buchteil wieder.
    const session = startPartSession(lastSessionPage.current);
    const result = await commands.ttsSpeakResume();
    if (result.status === "error") {
      endPartSession(session);
      setLastError(result.error);
    } else continueWithNextPart(session);
  };

  /**
   * The whole text — speaker changes and all — written to one WAV instead of
   * only played. Goes through the same segmentation as playback, so the file
   * sounds like what you heard.
   */
  /**
   * Dateiname des naechsten Exports: `<Stimme>[-Zusatz]_<Zeitstempel>.<ext>`.
   * Nicht der Seitentitel — der ist lang und verdraengt in der Dateileiste
   * den Zeitstempel; die Seite kennt man ohnehin am Projektordner. Die
   * Namensbildung selbst steht in `exportName.ts`, ohne Oberflaeche pruefbar.
   */
  const nextExportName = (ext: string) => {
    let voice: string;
    if (scriptMode) {
      voice = t("tts.export.scriptStem");
    } else if (voiceValue.startsWith("provider:")) {
      // "Phonon · Nele": nur der Name der Stimme gehoert in den Dateinamen.
      const label =
        voiceOptions.find((option) => option.value === voiceValue)?.label ?? "";
      voice = label.split(" · ").slice(1).join(" ") || label;
    } else if (voiceValue.startsWith("piper:")) {
      const piper = piperVoices.find((v) => `piper:${v.id}` === voiceValue);
      // Nur der Name: Sprache und Qualitaet der Auswahl-Beschriftung gehoeren
      // nicht in einen Dateinamen.
      voice = piper ? piperVoiceLabel(piper).split(" · ")[0] : "";
    } else {
      voice =
        fishSpeakers.find((sp) => sp.id === voiceValue)?.displayName ?? "";
    }
    const suffix =
      tab === "translation"
        ? targetLangCode(targetLang).toUpperCase()
        : tab === "summary"
          ? t("tts.export.summaryStem")
          : "";
    return audioExportName({
      voice,
      suffix,
      fallback: t("tts.export.untitled"),
      ext,
    });
  };

  const saveSpokenAudio = async () => {
    if (freshFindings().length > 0) {
      setCheckDialog("save");
      return;
    }
    await saveSpokenAudioNow();
  };

  const saveSpokenAudioNow = async () => {
    setLastError(null);
    // Der Speichern-Dialog schlaegt den Projektordner der Seite vor: dort
    // sammelt die Dateileiste rechts, was zu diesem Arbeitsblatt gehoert.
    // Ein anderer Ort bleibt jederzeit waehlbar.
    // Das Format steht in den Einstellungen — der Dialog fragt es nicht noch
    // einmal. Wer einmal MP3 gewaehlt hat, bekommt MP3 vorgeschlagen.
    // "opus" schlaegt bewusst eine .wav vor: einen Opus-Kodierer gibt es
    // nicht, der Export schreibt dafuer WAV-Daten — eine Datei namens .opus
    // mit WAV-Inhalt waere eine Luege ueber ihren eigenen Inhalt.
    const format = (getSetting("tts_export_format") ?? "wav").toLowerCase();
    const ext = format === "mp3" ? "mp3" : "wav";
    const filterName = ext === "mp3" ? "MP3" : "WAV";
    let defaultPath = nextExportName(ext);
    if (activePage) {
      const dir = await commands.pageDir(activePage);
      if (dir.status === "ok") {
        defaultPath = `${dir.data}\\${nextExportName(ext)}`;
      }
    }
    const target = await save({
      filters: [{ name: filterName, extensions: [ext] }],
      defaultPath,
    });
    if (typeof target !== "string") return;
    setSaving(true);
    setExportProgress({ position: 0, total: 0 });
    setExportEta(null);
    exportPerSentence.current = null;
    exportTick.current = { time: performance.now(), position: 0 };
    // Returns at once; the run reports itself through tts-export-progress.
    const result = await commands.ttsSpeakToFile(
      canonicalizeTags(toSpokenScript(spokenText)),
      target,
    );
    if (result.status === "error") {
      setSaving(false);
      setExportProgress(null);
      setExportEta(null);
      exportTick.current = null;
      setLastError(result.error);
      return;
    }
    // Der Befehl liefert den Pfad, der wirklich entsteht — bei MP3 mit
    // korrigierter Endung.
    exportPath.current = result.data;
  };

  const cancelExport = () => {
    void commands.ttsExportCancel();
  };

  /** One sentence back or forward — the unit spoken text moves in. */
  const seekSentence = (delta: number) => {
    // Springen bewegt die laufende Sitzung — auf einem anderen Reiter gibt
    // es nichts, worin man springen koennte.
    if (sessionTab.current !== tab) return;
    void commands.ttsSpeakSeek(delta);
  };

  // Waehrend des Serverstarts gibt es nichts fortzusetzen: der Lauf haengt noch im Start,
  // auch wenn sein Zaehler (`tts-speak-progress` zu Beginn) schon steht.
  const canResume =
    !speaking &&
    !starting &&
    speakProgress !== null &&
    speakProgress.position < speakProgress.total;
  // Zaehler "Satz n/m": der Satz, der zu hoeren ist (1-basiert) -- steht ab dem Start, nicht erst
  // nach dem ersten fertigen Satz.
  const sentenceCounter =
    speakProgress !== null
      ? (() => {
          const shown = shownSentenceNumber(
            currentIndex ?? speakProgress.position,
            speakProgress.total,
          );
          return shown === null
            ? null
            : { position: shown, total: speakProgress.total };
        })()
      : null;

  // Stimmenliste, Piper-Stimmen, Fish-Zustand und Stimmenwechsel kommen aus dem
  // Baustein, den auch die Wiedergabeleiste der Aufnahmen nutzt.
  const {
    fishReady,
    speakers: fishSpeakers,
    piperVoices,
    providerEngineId,
    providerLabel,
    voiceValue,
    scriptMode,
    applyVoiceValue,
    selectedVoiceNotSetUp,
    noVoiceSetUp,
    piperVoiceLabel,
    options: voiceOptions,
    sampleOf,
    tryOf,
  } = useVoiceChoice(phase);

  // Die Stimme gehoert zum Reiter -- bei der Uebersetzung zur Sprache: Englisch
  // liest Lessac, Franzoesisch eine andere, ohne neu zu waehlen.
  const voiceKey = tab === "translation" ? `translation:${targetLang}` : tab;
  // Seiten- oder Reiterwechsel (die Seite laedt `tabVoices` und `tab` aus ihrem
  // Zustand): die fuer diese Seite und diesen Reiter gemerkte Stimme wird aktiv.
  // Je (Seite, Reiter, Stimme) genau einmal: ein Neuaufbau von `applyVoiceValue`
  // (Einstellungen neu geladen) wiederholt eine fehlgeschlagene Wahl nicht.
  const appliedFor = useRef<string | null>(null);
  /** Die ausstehende Wahl dieses Effekts: wem sie gehoert, wenn `voiceValue` sie meldet. */
  const pendingApply = useRef<{
    value: string;
    page: string | null;
    key: string;
  } | null>(null);
  useEffect(() => {
    const wanted =
      tabVoices[voiceKey] ??
      (tab === "translation" ? tabVoices.translation : undefined);
    if (!wanted) return;
    const marker = `${activePage ?? ""}|${voiceKey}|${wanted}`;
    // Schon aktiv (z. B. gerade im Dropdown gewaehlt): als erledigt merken, damit
    // ein spaeteres Zurueckschalten auf diese Wahl nicht an einer alten Marke haengt.
    if (wanted === voiceValue) {
      appliedFor.current = marker;
      return;
    }
    if (appliedFor.current === marker) return;
    appliedFor.current = marker;
    pendingApply.current = {
      value: wanted,
      page: activePage ?? null,
      key: voiceKey,
    };
    applyVoiceValue(wanted);
    // voiceValue absichtlich nicht in den Abhaengigkeiten: der Effekt soll
    // beim Umschalten greifen, nicht bei jeder Einstellungsaenderung.
  }, [tab, voiceKey, tabVoices, applyVoiceValue, activePage]);
  // Jede Wahl (Dropdown, Wiedergabeleiste, Einstellungen) gilt fuer die
  // aktuelle Seite und den aktuellen Reiter: so kommt man beim Zurueckschalten
  // zu ihr zurueck. Meldet `voiceValue` die Wahl des Effekts oben (sie kommt
  // erst nach dem Neuladen der Einstellungen), gehoert sie dem Reiter, fuer den
  // sie angewendet wurde -- nicht dem, der inzwischen offen ist.
  useEffect(() => {
    if (!voiceValue || voiceValue === "@manage") return;
    const pending = pendingApply.current;
    let target = voiceKey;
    if (pending && pending.value === voiceValue) {
      pendingApply.current = null;
      if (pending.page !== (activePage ?? null)) return;
      target = pending.key;
    }
    setTabVoices((current) =>
      current[target] === voiceValue
        ? current
        : { ...current, [target]: voiceValue },
    );
    // voiceKey absichtlich nicht: beim Reiterwechsel ist voiceValue noch die
    // Stimme des alten Reiters.
  }, [voiceValue]);

  /**
   * Welcher Text gerade im Feld steht — und damit auch, was das Abspielen
   * spricht. Das Original wird NIE ueberschrieben: die Uebersetzung liegt
   * daneben, nicht darin.
   */
  const spokenText =
    tab === "original"
      ? text
      : tab === "translation"
        ? (translation ?? "")
        : summary;
  const setSpokenText = useCallback(
    (next: string) => {
      if (tab === "original") setText(next);
      else if (tab === "translation") setTranslation(next);
      else setSummary(next);
    },
    [tab],
  );
  // Buchansicht (lva#148): Lesen ist die Standardansicht, Bearbeiten der
  // Chip-Editor. Ohne Text gibt es nichts zu lesen — dann gilt Bearbeiten.
  const [viewModeStored, setViewMode] = usePersistentState<"read" | "edit">(
    "tts.view",
    "read",
    (v) => v === "read" || v === "edit",
  );
  // Wirksame Ansicht: beim Öffnen einer Seite (und nach dem Laden eines
  // Dokuments) gilt die gemerkte Wahl, solange es Text gibt. Tippen schaltet
  // NIE um — sonst verschwände der Editor beim ersten Buchstaben.
  const [modeNow, setModeNow] = useState<"read" | "edit">("edit");
  const viewMode: "read" | "edit" =
    modeNow === "read" && spokenText.trim().length > 0 ? "read" : "edit";
  const chooseViewMode = (mode: "read" | "edit") => {
    setViewMode(mode);
    setModeNow(mode);
  };
  openModeRef.current = (loaded: string) =>
    setModeNow(viewModeStored === "read" && loaded.trim() ? "read" : "edit");
  const [twoPagesStored, setTwoPages] = usePersistentState<"1" | "0">(
    "tts.reader.two",
    "1",
  );
  const [fontStored, setFontSize] = usePersistentState<string>(
    "tts.reader.font",
    "18",
    (v) => /^\d{2}$/.test(v),
  );
  const [speakersShown, setSpeakersShown] = usePersistentState<"1" | "0">(
    "tts.reader.speakers",
    "1",
  );
  const [fontFamily, setFontFamily] = usePersistentState<ReaderFont>(
    "tts.reader.family",
    "book",
    (v) => READER_FONTS.some((f) => f.value === v),
  );
  const [readerTheme, setReaderTheme] = usePersistentState<ReaderTheme>(
    "tts.reader.theme",
    "auto",
    (v) => READER_THEMES.some((th) => th.value === v),
  );
  // Sonnen-Knopf: bewusst nicht gemerkt — wach halten gilt nur, solange man
  // liest, und soll nach einem Neustart nicht still weiterlaufen.
  const [keepAwake, setKeepAwake] = useState(false);
  const awakeShift = useKeepAwake(keepAwake);
  const toggleKeepAwake = (on: boolean) => {
    setKeepAwake(on);
    // Im Vollbild ist nur die Lesespalte sichtbar, Meldungen nicht: der
    // Hinweis kommt beim nächsten Einschalten außerhalb des Vollbilds.
    if (on && !document.fullscreenElement)
      toast.info(t("tts.reader.keepAwakeHint"), { duration: 8000 });
  };
  const readerFrameRef = useRef<HTMLDivElement | null>(null);
  const [fullscreen, setFullscreen] = useState(false);
  useEffect(() => {
    const onChange = () =>
      setFullscreen(
        document.fullscreenElement !== null &&
          document.fullscreenElement === readerFrameRef.current,
      );
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, []);
  const toggleFullscreen = () => {
    if (document.fullscreenElement) {
      void document.exitFullscreen().catch(() => undefined);
      return;
    }
    void readerFrameRef.current?.requestFullscreen().catch(() => undefined);
  };
  const [follow, setFollow] = useState(true);
  const [pageInfo, setPageInfo] = useState<ReaderPageInfo | null>(null);
  const readerApiRef = useRef<ReaderViewApi | null>(null);
  const [revealed, setRevealed] = useState<SourceRange | null>(null);
  const readerBlocks = useMemo(
    () => parseReaderDoc(spokenText, speakers),
    [spokenText, speakers],
  );
  const locatorRef = useRef<SentenceLocator | null>(null);
  const locator = useMemo(() => new SentenceLocator(readerBlocks), [readerBlocks]);
  locatorRef.current = locator;
  const [readRange, setReadRange] = useState<SourceRange | null>(null);
  const readRangeRef = useRef<SourceRange | null>(null);
  readRangeRef.current = readRange;
  /** Zuletzt geortete Meldung; null nach Start einer neuen Sitzung. */
  const lastLocated = useRef<{
    sentence: string;
    index: number | null;
    locator: SentenceLocator;
  } | null>(null);
  useEffect(() => {
    if (!currentSentence || sessionTab.current !== tab) {
      if (!currentSentence) setReadRange(null);
      return;
    }
    const last = lastLocated.current;
    // Dieselbe Meldung (der Fortschritt setzt den Index nur zurück) nicht
    // erneut suchen: die Vorwärtssuche träfe sonst ein späteres Vorkommen.
    if (
      last &&
      last.locator === locator &&
      last.sentence === currentSentence &&
      (currentIndex === null || currentIndex === last.index)
    ) {
      return;
    }
    if (last && last.locator !== locator) {
      // Neuer Text oder Sprecher während der Wiedergabe: an der bisherigen
      // Stelle weitersuchen, nicht am Anfang.
      locator.reset(readRangeRef.current?.start ?? 0);
    } else if (
      last &&
      currentIndex !== null &&
      last.index !== null &&
      currentIndex < last.index
    ) {
      // Ein Sprung zurück: von vorn suchen, sonst träfe die Vorwärtssuche den
      // nächsten gleichlautenden Satz.
      locator.reset(0);
    }
    lastLocated.current = {
      sentence: currentSentence,
      index: currentIndex ?? last?.index ?? null,
      locator,
    };
    setReadRange(locator.locate(currentSentence));
  }, [currentSentence, currentIndex, locator, tab]);
  useEffect(() => {
    if (speakProgress === null && !speaking) setReadRange(null);
  }, [speakProgress, speaking]);
  // Weiterlesen (lva#148): die gelesene Stelle je Seite und Reiter merken.
  const resumeKey = `${activePage ?? "-"}:${
    tab === "translation" ? `translation:${targetLang}` : tab
  }`;
  const resumeKeyRef = useRef(resumeKey);
  resumeKeyRef.current = resumeKey;
  const sessionResumeKey = useRef<string | null>(null);
  const [resumeAt, setResumeAt] = useState<number | null>(null);
  useEffect(() => {
    setResumeAt(loadResumePoint(resumeKey, spokenText));
  }, [resumeKey, spokenText]);
  useEffect(() => {
    if (!readRange || sessionResumeKey.current !== resumeKeyRef.current) return;
    saveResumePoint(resumeKeyRef.current, spokenText, readRange.start);
    setResumeAt(loadResumePoint(resumeKeyRef.current, spokenText));
    // spokenText bewusst nicht: gespeichert wird beim Satzwechsel.
  }, [readRange]);
  const forgetResumePoint = (key: string = resumeKeyRef.current) => {
    clearResumePoint(key);
    if (key === resumeKeyRef.current) setResumeAt(null);
  };
  // Zu Ende gelesen: beim nächsten Play wieder von vorn.
  useEffect(() => {
    if (
      speakProgress &&
      speakProgress.total > 0 &&
      speakProgress.position >= speakProgress.total &&
      sessionResumeKey.current !== null
    ) {
      forgetResumePoint(sessionResumeKey.current);
    }
  }, [speakProgress]);
  // Seite der gemerkten Stelle; pageInfo löst nach jedem Umbruch neu aus.
  const resumePage = useMemo(() => {
    if (resumeAt === null || pageInfo === null) return null;
    const p = readerApiRef.current?.pageOfOffset(resumeAt);
    return p === null || p === undefined ? null : p + 1;
  }, [resumeAt, pageInfo]);
  // Inhaltsverzeichnis: Überschriften der offenen Seite; aktuelles Kapitel
  // nach der gelesenen Stelle, sonst nach der sichtbaren Seite.
  const contents = useMemo(() => buildContents(readerBlocks), [readerBlocks]);
  const book = useMemo(() => bookInfo(readerBlocks), [readerBlocks]);
  // Buchdaten per ISBN (lva#149): Vorschlag aus dem Impressum, Abfrage erst
  // im Dialog auf Knopfdruck, Übernahme erst nach Bestätigung.
  const [isbnOpen, setIsbnOpen] = useState(false);
  const isbnGuess = useMemo(
    () => (isbnOpen ? findIsbn(text) : ""),
    [isbnOpen, text],
  );
  // Nach dem Commit messen: im Render stünde noch das DOM des alten Umbruchs.
  const [chapterIndex, setChapterIndex] = useState(-1);
  const [contentsPages, setContentsPages] = useState<(number | null)[]>([]);
  useEffect(() => {
    const api = readerApiRef.current;
    setContentsPages(contents.map((e) => api?.pageOfOffset(e.start) ?? null));
    if (contents.length === 0) {
      setChapterIndex(-1);
      return;
    }
    const reading =
      readRange && sessionResumeKey.current === resumeKey ? readRange.start : null;
    const shown =
      pageInfo !== null ? (api?.offsetOfPage(pageInfo.page) ?? null) : null;
    const at = reading ?? shown;
    setChapterIndex(at === null ? -1 : chapterAt(contents, at));
  }, [contents, readRange, pageInfo, resumeKey]);
  const goToChapter = (entry: ContentsEntry) => {
    setFollow(false);
    readerApiRef.current?.revealOffset(entry.start);
  };
  // Lesezeichen je Seite und Reiter.
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([]);
  useEffect(() => {
    setBookmarks(loadBookmarks(resumeKey, spokenText));
  }, [resumeKey, spokenText]);
  const addBookmarkHere = () => {
    // Beim Vorlesen die gelesene Stelle, sonst der Anfang der sichtbaren Seite.
    const reading =
      readRange && sessionResumeKey.current === resumeKey ? readRange.start : null;
    const shown = pageInfo
      ? readerApiRef.current?.offsetOfPage(pageInfo.currentPage ?? pageInfo.page)
      : null;
    const at = reading ?? shown ?? 0;
    addBookmark(resumeKey, spokenText, at);
    setBookmarks(loadBookmarks(resumeKey, spokenText));
    toast.success(t("tts.reader.bookmarkSet"));
  };
  const goToBookmark = (b: Bookmark) => {
    // Aufschlagen und als Weiterlesen-Stelle merken: Play liest ab dort.
    setFollow(false);
    readerApiRef.current?.revealOffset(b.offset);
    // Ein Lesezeichen am Anfang heißt: Play beginnt vorn.
    if (b.offset <= 0) {
      forgetResumePoint(resumeKey);
      return;
    }
    saveResumePoint(resumeKey, spokenText, b.offset);
    setResumeAt(loadResumePoint(resumeKey, spokenText));
  };
  const dropBookmark = (b: Bookmark) => {
    removeBookmark(resumeKey, b.id);
    setBookmarks(loadBookmarks(resumeKey, spokenText));
  };
  // Beim Öffnen einer Seite die gemerkte Stelle aufschlagen.
  const revealedResumeFor = useRef<string | null>(null);
  useEffect(() => {
    if (viewMode !== "read" || !spokenText || pageInfo === null) return;
    if (revealedResumeFor.current === resumeKey) return;
    const at = loadResumePoint(resumeKey, spokenText);
    if (at === null) {
      revealedResumeFor.current = resumeKey;
      return;
    }
    // Erst als erledigt merken, wenn der Reader die Stelle wirklich zeigt:
    // ein abgebrochener Frame oder ein noch fehlender Reader versucht es neu.
    const id = window.requestAnimationFrame(() => {
      const api = readerApiRef.current;
      if (!api) return;
      api.revealOffset(at);
      revealedResumeFor.current = resumeKey;
    });
    return () => window.cancelAnimationFrame(id);
  }, [resumeKey, spokenText, viewMode, pageInfo]);
  /** Geschätzte Lesedauer bei Tempo 1: ~14 Zeichen je Sekunde. */
  const readEstimate = useMemo(
    () => Math.max(1, toSpokenScript(spokenText).replace(/<[^<>\n]{1,60}>|\[[^\]\n]*\]/g, "").length / 14),
    [spokenText],
  );
  const historyRef = useRef<{
    syncPresent: (key: string, value: string) => void;
  } | null>(null);
  const activePageRef = useRef<string | null>(null);
  activePageRef.current = activePage ?? null;

  // Buchteile (lva#150): Gehört die Seite zu einem Buch der Skript-Werkstatt,
  // liest das Vorlesen am Ende im nächsten Teil weiter — ohne Abriss, weil
  // die ersten Sätze des nächsten Teils schon vorberechnet sind, während der
  // laufende Teil ausklingt.
  const [books, setBooks] = useState<Book[]>([]);
  useEffect(() => {
    let live = true;
    commands
      .booksList()
      .then((r) => {
        if (live && r.status === "ok" && Array.isArray(r.data)) setBooks(r.data);
      })
      .catch(() => {
        /* ohne Bücherliste liest das Vorlesen nur den einen Teil */
      });
    return () => {
      live = false;
    };
  }, [pages]);
  const nextPart = useMemo(() => {
    if (!activePage) return null;
    for (const b of books) {
      const i = b.page_ids.indexOf(activePage);
      if (i < 0) continue;
      for (const id of b.page_ids.slice(i + 1)) {
        const page = pages.find((p) => p.id === id);
        if (page) return { id, title: page.title };
      }
    }
    return null;
  }, [books, activePage, pages]);
  const nextPartRef = useRef(nextPart);
  nextPartRef.current = nextPart;
  const tabRef = useRef(tab);
  tabRef.current = tab;
  /** Seite, deren Sitzung gerade läuft, und ob ihr Anlauf schon angefragt ist.
   *  Jede Sitzung hat ihr eigenes Objekt: eine später endende alte Sitzung
   *  darf die neue nicht abräumen. */
  type PartSession = { page: string; prefetched: boolean };
  const partSession = useRef<PartSession | null>(null);
  /** Seite der zuletzt gestarteten Sitzung — Play nach Pause setzt sie fort. */
  const lastSessionPage = useRef<string | null>(null);
  /** Nach dem Seitenwechsel sofort weiterlesen (Seite, auf die gewechselt wurde). */
  const autoplayPage = useRef<string | null>(null);

  const startPartSession = (
    page: string | null = activePageRef.current,
  ): PartSession | null => {
    lastSessionPage.current = page;
    const session = page ? { page, prefetched: false } : null;
    partSession.current = session;
    return session;
  };
  const endPartSession = (session: PartSession | null) => {
    if (session && partSession.current === session) partSession.current = null;
  };

  // Die letzten Sätze laufen: den Anfang des nächsten Teils still vorberechnen.
  // Nur im Original — Übersetzung und Zusammenfassung lesen nicht weiter.
  useEffect(() => {
    const session = partSession.current;
    if (!session || session.prefetched || !nextPart || !speakProgress) return;
    if (session.page !== activePageRef.current) return;
    if (sessionTab.current !== "original" || tab !== "original") return;
    const { position, total } = speakProgress;
    if (total === 0 || position < Math.max(0, total - PART_LEAD_SENTENCES))
      return;
    session.prefetched = true;
    commands
      .pageStateLoad(nextPart.id)
      .then((r) => {
        // Inzwischen gestoppt oder neu gestartet: nichts mehr vorrechnen.
        if (partSession.current !== session) return;
        if (r.status !== "ok" || !r.data) return;
        let nextText = r.data;
        try {
          nextText = (JSON.parse(r.data) as { text?: string }).text ?? "";
        } catch {
          // Ältere Seiten speichern den Text roh.
        }
        if (!nextText.trim()) return;
        void commands.ttsPrefetchHead(
          canonicalizeTags(toSpokenScript(nextText)),
        );
      })
      .catch(() => {
        /* ohne Anlauf liest der nächste Teil trotzdem, nur mit Pause */
      });
  }, [speakProgress, nextPart, tab]);

  /**
   * Nach dem Ende einer Sitzung: war sie vollständig (letzter Satz gehört),
   * ist sie noch die aktuelle und gibt es einen nächsten Teil, dorthin
   * wechseln und weiterlesen.
   */
  const continueWithNextPart = (session: PartSession | null) => {
    if (!session || partSession.current !== session) return;
    partSession.current = null;
    const progress = speakProgressRef.current;
    const next = nextPartRef.current;
    if (!progress || progress.total === 0 || progress.position < progress.total)
      return;
    if (activePageRef.current !== session.page || !next) return;
    if (sessionTab.current !== "original" || tabRef.current !== "original")
      return;
    autoplayPage.current = next.id;
    setActivePage(next.id);
    toast(t("tts.parts.next", { title: next.title }));
  };
  // Woanders hingewechselt, bevor der nächste Teil geladen war: kein
  // Weiterlesen mehr, auch nicht, wenn die Seite später von Hand geöffnet wird.
  useEffect(() => {
    if (autoplayPage.current && autoplayPage.current !== activePage)
      autoplayPage.current = null;
  }, [activePage]);
  // Bibliothek (lva#152): „Weiterhören“ öffnet die Seite und liest sofort.
  useEffect(() => {
    const page = takeLibraryAutoplay();
    if (page && page === activePage) autoplayPage.current = page;
    // Nur beim Öffnen von Vorlesen.
  }, []);
  // Der nächste Teil ist geladen: vorlesen. `pageLoadTick` zählt jedes
  // fertige Laden mit — sonst liefe das bei gleichem Text nie an.
  useEffect(() => {
    if (!autoplayPage.current || autoplayPage.current !== activePage) return;
    if (!pageLoaded.current) return;
    autoplayPage.current = null;
    // Nur im Originaltext und nur, wenn es etwas zu lesen gibt: eine
    // Übersetzung oder Zusammenfassung des nächsten Teils liest niemand
    // ungefragt vor.
    if (tab !== "original" || !spokenText.trim()) return;
    void speakNow();
    // speakNow liest den aktuellen Stand; neu laufen soll das nur, wenn der
    // gewechselte Teil fertig geladen ist.
  }, [pageLoadTick, activePage, tab, spokenText]);
  // Bearbeitungshistorie je Seite und Reiter (Uebersetzung: je Sprache): Rueckgaengig/Wiederherstellen
  // ueber Knoepfe und Strg+Z/Strg+Y -- auch fuer programmatische
  // Aenderungen (Auto-Tagging, Aufbereiten, Ersetzen, Skript-Pruefung).
  const history = useTextHistory(
    tab === "translation"
      ? translationHistoryKey(activePage, targetLang)
      : `${activePage ?? "-"}:${tab}`,
    spokenText,
    setSpokenText,
  );

  historyRef.current = history;

  /**
   * Skript-Pruefung des aktiven Reiters: Sprechermarker ohne Stimme und
   * Tags, die die aktive Engine nicht kennt. Ist der Schalter aus, wird
   * nicht geprueft und nichts angezeigt -- die Liste ist dann leer.
   */
  // Automatik (Schalter, Standard an): waehrend der Eingabe pruefen und im
  // Text unterstreichen -- wie die Rechtschreibpruefung in Office. Das
  // Korrekturpanel oeffnet erst per Knopf "Skript pruefen"; der Knopf
  // prueft auch bei abgeschaltetem Schalter (dann einmalig fuer diesen
  // Text). Die Analyse laeuft ueber den um 120 ms verzoegerten Text, damit
  // die Textarea nie auf sie wartet (Zweitmeinung 21.09.2026: Debounce +
  // Zwei-Zeiger-Zuordnung reichen fuer 100 KB; Zeilen-Cache nur bei Bedarf).
  const [manualCheck, setManualCheck] = useState<string | null>(null);
  // Das Panel bleibt offen, bis man es schliesst -- auch ueber Seiten- und
  // Reiterwechsel hinweg (localStorage: Bequemlichkeit je Rechner).
  const [panelOpen, setPanelOpenState] = useState<boolean>(() => {
    try {
      return window.localStorage.getItem("lv-script-check-panel") === "1";
    } catch {
      return false;
    }
  });
  const setPanelOpen = (open: boolean) => {
    setPanelOpenState(open);
    try {
      window.localStorage.setItem("lv-script-check-panel", open ? "1" : "0");
    } catch {
      // egal
    }
  };
  const scriptCheckAuto = getSetting("tts_script_check") ?? true;
  const scriptCheckOn = scriptCheckAuto || manualCheck === spokenText;
  const scriptEngine = engineCaps.scriptEngine;
  const debouncedText = useDebouncedValue(spokenText, 120);
  const scriptFindings = useMemo<ScriptFinding[]>(
    () =>
      scriptCheckOn ? checkScript(debouncedText, speakers, scriptEngine) : [],
    [scriptCheckOn, debouncedText, speakers, scriptEngine],
  );

  const editorFindings = useMemo<ChipEditorFinding[] | undefined>(
    () =>
      scriptCheckOn
        ? scriptFindings
            // Schreibweisen-Vorschlaege sind kein Fehler: kein Rot im Text.
            .filter((f) => !isHintFinding(f.kind))
            .map((f) => ({
              start: f.start,
              end: f.end,
              message:
                f.kind === "unknown-speaker"
                  ? t("tts.scriptCheck.unknownSpeaker", { name: f.name })
                  : t("tts.scriptCheck.unknownTag", { tag: f.name }),
            }))
        : undefined,
    [scriptCheckOn, scriptFindings, t],
  );
  /** Massnahmen der Befundliste. "Ueberall" ersetzt den ganzen Text des
   *  Reiters in EINEM Undo-Schritt ueber die Editor-API; ohne Editor-API
   *  (sollte nicht vorkommen) faellt es auf setState zurueck. */
  /** Befunde des aktuellen (nicht verzoegerten) Texts, nur mit Schalter an. */
  const freshFindings = (): ScriptFinding[] =>
    scriptCheckAuto
      ? checkScript(spokenText, speakers, scriptEngine).filter(
          (f) => !isHintFinding(f.kind),
        )
      : [];
  const errorCount = scriptFindings.filter(
    (f) => !isHintFinding(f.kind),
  ).length;

  const runScriptCheck = () => {
    setManualCheck(spokenText);
    const first = checkScript(spokenText, speakers, scriptEngine)[0];
    setPanelOpen(true);
    if (first) editorApiRef.current?.revealRange?.(first.start, first.end);
  };

  /** Die Befunde im AKTUELLEN Text (die angezeigte Liste haengt 120 ms
   *  hinterher) und darin die Entsprechung des angeklickten Befunds: gleiche
   *  Art, gleicher Name, naechstgelegene Stelle. Ohne diesen Abgleich
   *  entfernte "Nur diese entfernen" direkt nach einem "Alle ersetzen" die
   *  falschen Zeichen (26.09.2026, Test "replace all swaps…"). */
  const freshMatch = useCallback(
    (finding: ScriptFinding) => {
      const fresh = checkScript(spokenText, speakers, scriptEngine);
      const candidates = fresh.filter(
        (f) =>
          f.kind === finding.kind &&
          f.name.toLowerCase() === finding.name.toLowerCase(),
      );
      const target = candidates.reduce<ScriptFinding | null>(
        (best, f) =>
          !best ||
          Math.abs(f.start - finding.start) <
            Math.abs(best.start - finding.start)
            ? f
            : best,
        null,
      );
      return { fresh, target };
    },
    [spokenText, speakers, scriptEngine],
  );

  /**
   * Befunde in der Buchansicht: Sprecher-Marker sind dort unsichtbar (sie
   * stehen am Rand). Markiert wird deshalb der Text, den der Marker spricht —
   * der Block, der direkt hinter ihm beginnt.
   */
  const readerMarks = useMemo<ReaderMark[]>(() => {
    if (!scriptCheckOn) return [];
    const out: ReaderMark[] = [];
    for (const f of scriptFindings) {
      if (isHintFinding(f.kind)) continue;
      const block = readerBlocks.find((b) => b.end > f.start && b.runs.length);
      const runs = block?.runs.filter((r) => !r.tag) ?? [];
      if (!runs.length) continue;
      const start = f.kind === "unknown-speaker" ? runs[0].start : f.start;
      const end = f.kind === "unknown-speaker" ? runs[runs.length - 1].end : f.end;
      out.push({
        start,
        end,
        selected:
          revealed !== null &&
          revealed.start <= f.end &&
          revealed.end >= f.start,
      });
    }
    return out;
  }, [scriptCheckOn, scriptFindings, readerBlocks, revealed]);

  const scriptActions = useMemo(
    () => ({
      reveal: (finding: ScriptFinding) => {
        const { target } = freshMatch(finding);
        const at = target ?? finding;
        editorApiRef.current?.revealRange?.(at.start, at.end);
        setRevealed({ start: at.start, end: at.end });
        setFollow(false);
        readerApiRef.current?.revealOffset(at.end);
      },
      replace: (
        finding: ScriptFinding,
        replacement: (f: ScriptFinding) => string,
        everywhere: boolean,
      ) => {
        const api = editorApiRef.current;
        const { fresh, target } = freshMatch(finding);
        if (!target) return;
        if (everywhere) {
          const next = replaceFindingEverywhere(
            spokenText,
            fresh,
            target,
            replacement,
          );
          if (next === spokenText) return;
          if (api?.replaceRange) {
            api.replaceRange(0, spokenText.length, next);
          } else if (tab === "original") {
            setText(next);
          } else if (tab === "translation") {
            setTranslation(next);
          } else {
            setSummary(next);
          }
          return;
        }
        api?.replaceRange?.(target.start, target.end, replacement(target));
      },
    }),
    [spokenText, freshMatch, tab],
  );

  /**
   * Uebersetzen — nur uebersetzen. Kein Abspielen, kein Aufnehmen.
   *
   * Das Ergebnis liegt im Zwischenspeicher des Backends, je Originaltext UND
   * Sprache. Zwischen zwei Sprachen hin und her zu wechseln kostet nach dem
   * ersten Mal nichts mehr, auch nach einem Neustart nicht.
   */
  const translateText = async () => {
    if (!text.trim()) return;
    setLastError(null);
    setTranslating(true);
    setTransformProgress(null);
    const result = await commands.ttsTranslate(stripPageMarks(text), targetLang);
    setTranslating(false);
    setTransformProgress(null);
    if (result.status === "error") {
      if (result.error === TRANSFORM_CANCELLED) {
        toast.info(t("tts.transform.cancelled"));
      } else {
        setLastError(result.error);
      }
      return;
    }
    translationFor.current = translationKey(text, targetLang);
    savedTranslation.current = result.data;
    addTranslatedLang(targetLang);
    setTranslation(result.data);
    setTab("translation");
    // Die Sprecher des Originals stehen noch drin: Stimmen der Zielsprache
    // vorschlagen, sobald es Sprecher gibt.
    if (speakersInText(result.data, speakers).length > 0) {
      void openSpeakerSwap(result.data);
    }
  };

  const piperAutoLanguage = getSetting("tts_piper_auto_language") ?? true;
  useEffect(() => {
    if (!voiceValue.startsWith("piper:") || !piperAutoLanguage) {
      setPiperReader(null);
      return;
    }
    let alive = true;
    const timer = window.setTimeout(() => {
      void commands
        .ttsPiperVoiceForText(spokenText)
        .then((id) => alive && setPiperReader(id))
        .catch(() => alive && setPiperReader(null));
    }, 400);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [voiceValue, piperAutoLanguage, spokenText]);
  const piperReaderVoice = piperReader
    ? piperVoices.find((v) => v.id === piperReader)
    : undefined;

  // Korrekturen an der Uebersetzung bleiben: nach kurzer Ruhe in den
  // Zwischenspeicher schreiben, aus dem sie beim Wechsel von Sprache oder
  // Seite wieder geladen wird. Nur, wenn der Text zu GENAU diesem Original und
  // dieser Sprache gehoert -- gleich nach einem Sprachwechsel steht noch die
  // alte Uebersetzung im Feld und darf nicht unter der neuen Sprache landen.
  useEffect(() => {
    if (translation === null) return;
    if (translationFor.current !== translationKey(text, targetLang)) return;
    if (translation === savedTranslation.current) return;
    const forText = text;
    const forLang = targetLang;
    const value = translation;
    const timer = window.setTimeout(() => {
      void commands.ttsStoreTranslation(forText, forLang, value).then(() => {
        savedTranslation.current = value;
      });
    }, 600);
    return () => window.clearTimeout(timer);
  }, [translation, text, targetLang]);

  const swapKey = `lva.ui.speakerSwap.${activePage ?? ""}.${targetLang}`;
  /** Fenster "Sprecher tauschen" mit Vorbelegung (gemerkt je Seite und Sprache). */
  const openSpeakerSwap = async (source: string = translation ?? "") => {
    let voices: LangVoice[];
    if (engineCaps.kind === "provider") {
      // Liest ein Anbieter vor, sind die Sprecher SEINE Stimmen (mit Sprache und
      // Geschlecht), nicht die aufgenommenen Fish-Stimmen.
      const listed = await commands.ttsProviderSpeakers().catch(() => null);
      voices = providerSpeakerRefs(
        listed?.status === "ok" && Array.isArray(listed.data)
          ? listed.data
          : [],
      ).map((voice) => ({ ...voice, language: voice.language ?? "" }));
    } else {
      const infos = await commands.ttsListVoiceInfos().catch(() => []);
      voices = infos.map((info) => ({
        id: info.id,
        displayName: info.meta.display_name,
        color: voiceColor(info.meta.color),
        language: info.meta.language ?? "",
      }));
    }
    setLangVoices(voices);
    const sources = speakersInText(source, voices);
    if (sources.length === 0) {
      toast.info(t("tts.speakerSwap.none"));
      return;
    }
    let stored: Record<string, string> = {};
    try {
      stored = JSON.parse(localStorage.getItem(swapKey) ?? "{}");
    } catch {
      stored = {};
    }
    const suggested = suggestSwap(sources, voices, targetLangCode(targetLang));
    const initial: Record<string, string> = {};
    for (const s of sources) {
      const keep = stored[s.id];
      initial[s.id] =
        keep && voices.some((v) => v.id === keep) ? keep : suggested[s.id];
    }
    setSwapInitial(initial);
    setSwapOpen(true);
  };

  const applySpeakerSwap = (mapping: Record<string, string>) => {
    try {
      localStorage.setItem(swapKey, JSON.stringify(mapping));
    } catch {
      /* gemerkt wird nur zur Bequemlichkeit */
    }
    setTranslation((current) => applySwap(current ?? "", mapping, langVoices));
    // Welche Stimme liest, bleibt die Wahl des Nutzers im Stimmen-Menü; bei
    // "Skript mit Sprechern" gelten die getauschten Marken, sonst liest die
    // gewählte Stimme alles.
  };

  /**
   * T4 Auto-Tagging: Tags wirklich in den Text übernehmen (Annehmen, einzeln
   * oder "Alle annehmen") — mit Undo-Toast, der den Vortext wiederherstellt.
   * Reines Verwerfen ruft das NICHT auf (der Text ändert sich dabei nicht).
   * Zieht `tagSuggestionsSourceText` auf den neuen Text nach — die noch
   * offenen (nachgezogenen) Vorschläge gelten jetzt gegen DIESEN Text.
   */
  const applyAutoTagText = (
    nextText: string,
    previousText: string,
    count: number,
  ) => {
    setText(nextText);
    setTagSuggestionsSourceText(nextText);
    toast(t("tts.autotag.appliedToast", { count }), {
      action: {
        label: t("tts.autotag.undo"),
        onClick: () => setText(previousText),
      },
    });
  };

  /**
   * An `TtsChipEditor.onResolveSuggestion` gereicht: Annehmen/Verwerfen EINES
   * Vorschlags aus dessen Popover (Check/X-Buttons). Review-Befund 2: wurde
   * der Text seit der letzten Anwendung editiert (`tagSuggestionsSourceText`
   * weicht ab), würde Annehmen blind an einem möglicherweise falschen Offset
   * splicen — dann wird der betroffene Vorschlag stattdessen verworfen.
   * Verwerfen selbst ist davon nie betroffen: es ändert den Text nicht.
   */
  const resolveTagSuggestion = (id: string, accept: boolean) => {
    if (
      accept &&
      tagSuggestionsSourceText !== null &&
      text !== tagSuggestionsSourceText
    ) {
      setTagSuggestions((prev) => prev.filter((s) => s.id !== id));
      toast.info(t("tts.autotag.staleSuggestionsDiscarded"));
      return;
    }
    const outcome = resolveSuggestion(
      text,
      tagSuggestions,
      id,
      accept,
      tagLang,
    );
    setTagSuggestions(outcome.suggestions);
    if (outcome.inserted) {
      applyAutoTagText(outcome.text, text, 1);
    }
  };

  /** An `AutoTagBar.onSuggestionsChange` gereicht — hält Vorschlagsliste und
   *  ihren Referenztext (`tagSuggestionsSourceText`) synchron. */
  const changeTagSuggestions = (
    next: ChipEditorSuggestion[],
    sourceText: string | null,
  ) => {
    setTagSuggestions(next);
    setTagSuggestionsSourceText(sourceText);
  };

  /**
   * Diktieren — nur diktieren. Der erkannte Text landet im Feld; was damit
   * geschieht, entscheidet danach der Nutzer.
   */
  const toggleDictation = async () => {
    setLastError(null);
    if (dictating) {
      setDictating(false);
      const result = await commands.ttsDictateStop();
      if (result.status === "error") {
        setLastError(result.error);
        return;
      }
      // An den vorhandenen Text anhaengen statt ihn zu ersetzen: Diktieren
      // ist Weiterschreiben, nicht Neuanfangen.
      setTab("original");
      setText(text.trim() ? `${text.trim()}\n\n${result.data}` : result.data);
      return;
    }
    const result = await commands.ttsDictateStart();
    if (result.status === "error") {
      setLastError(result.error);
      return;
    }
    setDictating(true);
  };

  /** Dokument als Originaltext laden (txt/md/pdf/docx/epub). */
  const loadDocument = async () => {
    const picked = await open({
      multiple: false,
      filters: [
        { name: "Dokumente", extensions: ["txt", "md", "pdf", "docx", "epub"] },
      ],
    });
    if (typeof picked !== "string") return;
    setLastError(null);
    setLoadingSource(true);
    const wasEmpty = !text.trim();
    const pageAtStart = activePage;
    const result = await commands.ttsExtractDocument(picked);
    setLoadingSource(false);
    if (result.status === "error") {
      setLastError(result.error);
      return;
    }
    const title = documentTitle(result.data, picked);
    // Buch hinzufügen (lva#152): steht auf der Seite schon Text, bekommt das
    // Buch eine eigene neue Seite – nichts wird überschrieben.
    if (!wasEmpty) {
      await createPageWithText(title, result.data);
      return;
    }
    setTab("original");
    setText(result.data);
    // Auf einer leeren Seite heißt die Seite wie das Buch (lva#149), aber
    // nur, wenn der Nutzer während des Ladens nicht die Seite gewechselt hat.
    if (pageAtStart && activePageRef.current === pageAtStart && title) {
      void commands
        .pagesRename(pageAtStart, title)
        .then(() => reloadPages());
    }
    openModeRef.current?.(result.data);
  };

  /** Bestätigte ISBN-Daten in die Titelei schreiben, Seite umbenennen. */
  const applyIsbnBook = (found: IsbnBook) => {
    const previousText = text;
    const next = applyBookTitle(text, found);
    if (next === previousText) return;
    setText(next);
    const page = activePage;
    const oldTitle = pages.find((p) => p.id === page)?.title;
    if (page) {
      const title = found.title.replace(/\s+/g, " ").trim().slice(0, 120);
      void commands.pagesRename(page, title).then(() => reloadPages());
    }
    toast(t("tts.isbn.applied", { title: found.title }), {
      action: {
        label: t("tts.autotag.undo"),
        onClick: () => {
          // Nur auf derselben Seite: sonst landete der alte Text per
          // Autosave auf der jetzt offenen Seite.
          if (activePageRef.current !== page) return;
          setText(previousText);
          if (page && oldTitle)
            void commands.pagesRename(page, oldTitle).then(() => reloadPages());
        },
      },
    });
  };

  /** Artikel hinter einer URL als Originaltext laden. */
  const loadUrl = async () => {
    if (!sourceUrl.trim()) return;
    setLastError(null);
    setLoadingSource(true);
    const result = await commands.ttsExtractUrl(sourceUrl.trim());
    setLoadingSource(false);
    if (result.status === "error") {
      setLastError(result.error);
      return;
    }
    setTab("original");
    setText(result.data);
    openModeRef.current?.(result.data);
  };

  /** Beliebige Datei in den Projektordner der Seite kopieren — Kontext, der
   *  nicht in das Textfeld gehoert (Audio, PDF-Original, Notizen). */
  const addFileToProject = async () => {
    if (!activePage) return;
    const picked = await open({ multiple: false });
    if (typeof picked !== "string") return;
    setLastError(null);
    const result = await commands.pageFileAdd(activePage, picked);
    if (result.status === "error") {
      setLastError(result.error);
      return;
    }
    window.dispatchEvent(new CustomEvent("lv-files-changed"));
  };

  /**
   * Text aufbereiten: Seitenzahlen, Kopf-/Fusszeilen, Trennungen und harte
   * Umbrueche raus, Absaetze zusammensetzen -- ohne Inhalt zu verlieren. Fuer
   * Dokumente und Zwischenablage, die nie fuers Vorlesen gedacht waren. Der
   * alte Text bleibt hinter "Rueckgaengig" im Toast.
   */
  const tidyText = async () => {
    if (!text.trim() || tidying) return;
    setLastError(null);
    setTidying(true);
    const previousText = text;
    const result = await commands.ttsTidyText(stripPageMarks(text));
    setTidying(false);
    if (result.status === "error") {
      setLastError(result.error);
      return;
    }
    setTab("original");
    setText(result.data);
    toast(t("tts.tidyDone"), {
      action: {
        label: t("tts.tidyUndo"),
        onClick: () => setText(previousText),
      },
    });
  };

  /**
   * Zusammenfassen — immer das ORIGINAL, nie die Uebersetzung. Das Ergebnis
   * liegt im dritten Reiter; das Original bleibt unangetastet, abspielen
   * kann man beides.
   */
  const summarize = async () => {
    if (!text.trim()) return;
    setLastError(null);
    setSummarizing(true);
    setTransformProgress(null);
    const result = await commands.ttsSummarizeText(stripPageMarks(text), {
      length: sumLength,
      detail: sumDetail,
      audience: sumAudience,
    });
    setSummarizing(false);
    setTransformProgress(null);
    if (result.status === "error") {
      if (result.error === TRANSFORM_CANCELLED) {
        toast.info(t("tts.transform.cancelled"));
      } else {
        setLastError(result.error);
      }
      return;
    }
    setSummary(result.data);
    setTab("summary");
  };

  const showVramHint =
    starting && (startingSeconds >= 120 || status?.message === "vram");

  // Stimme und Werkzeuge: rechts in der Spalte oder, ist sie ausgeblendet,
  // in der Spielerleiste (Variante A, Codex 01.11.). Gerendert wird jeweils
  // nur an einer Stelle.
  // `compact`: in der Spielerleiste nur der Dropdown-Knopf (Symbol + Pfeil),
  // der Name steht im Tooltip und in der offenen Liste (Patrick 01.11.).
  const voiceLabel =
    voiceOptions.find((option) => option.value === voiceValue)?.label ?? "";
  const voicePickerOf = (compact = false) => (
      <TooltipTrigger
        className="block w-full"
        tooltipId="voice-mode-tip"
        width={300}
        content={
          <div className="text-xs">
            {scriptMode
              ? t("tts.voices.scriptVoicesHint")
              : t("tts.voices.singleVoiceHint")}
          </div>
        }
      >
        <div
          className="w-full"
          role="group"
          aria-label={t("tts.voices.title")}
          aria-describedby="voice-mode-hint"
          data-testid="voice-select"
          onContextMenu={(event) =>
            voiceMenu.open(event, [
              {
                id: "replace",
                label: t("tts.replaceSpeaker.voiceMenu"),
                icon: Replace,
                testId: "voice-menu-replace",
                disabled: speakers.length < 2,
                onSelect: () => setReplaceSpeaker({}),
              },
              {
                id: "manage",
                label: t("tts.voices.manage"),
                icon: Settings,
                testId: "voice-menu-manage",
                onSelect: () => {
                  window.localStorage.setItem(
                    "lva.ui.settings.tab",
                    "output",
                  );
                  window.dispatchEvent(
                    new CustomEvent("lv-navigate", {
                      detail: { section: "settings" },
                    }),
                  );
                },
              },
            ])
          }
        >
          {/* Kennwert statt leerem Text fuer die Standardstimme:
            "" gilt der Select-Komponente als "nichts gewaehlt" und
            zeigte den Platzhalter "Select…" statt des Namens.
            Piper-Stimmen stehen in derselben Liste (Wert
            "piper:<id>"): wer eine waehlt, schaltet damit die
            Engine um — die Engine-Einstellung im Reiter Vorlesen
            bleibt als zweiter Weg bestehen. */}
          <Select
            menuPortal
            compactMenuWidth={compact ? 300 : undefined}
            ariaLabel={compact ? `${t("tts.voices.title")}: ${voiceLabel}` : undefined}
            value={voiceValue}
            options={voiceOptions}
            formatOptionLabel={(option, context) => {
              if (compact && context === "value")
                return (
                  <span
                    className="flex items-center"
                    title={`${t("tts.voices.title")}: ${option.label}`}
                    data-testid="voice-compact"
                  >
                    <Mic width={16} height={16} aria-hidden="true" />
                    <span className="sr-only">{option.label}</span>
                  </span>
                );
              const sample =
                context === "menu"
                  ? sampleOf(option.value)
                  : undefined;
              const target =
                context === "menu" && !sample
                  ? tryOf(option.value)
                  : undefined;
              if (!sample && !target) return option.label;
              return (
                <span className="flex items-center gap-2">
                  <span className="min-w-0 flex-1 truncate">
                    {option.label}
                  </span>
                  {sample ? (
                    <SamplePlayButton
                      inline
                      path={sample.path}
                      name={sample.name}
                      testId={`voice-sample-${option.value}`}
                    />
                  ) : (
                    target && (
                      <VoiceTryButton
                        target={target}
                        name={option.label}
                        testId={`voice-sample-${option.value}`}
                      />
                    )
                  )}
                </span>
              );
            }}
            onChange={(value) => {
              if (!value) return;
              if (value === "@manage") {
                window.localStorage.setItem(
                  "lva.ui.settings.tab",
                  "output",
                );
                window.dispatchEvent(
                  new CustomEvent("lv-navigate", {
                    detail: { section: "settings" },
                  }),
                );
                return;
              }
              applyVoiceValue(value);
              setTabVoices((current) => ({
                ...current,
                [voiceKey]: value,
              }));
            }}
            isClearable={false}
          />
          {providerEngineId && (
            <div
              role="status"
              data-testid="voice-provider-hint"
              className="mt-1 text-xs text-text/70"
            >
              <p>
                {t("tts.providers.dropdown.hint", {
                  name: providerLabel ?? providerEngineId,
                })}
              </p>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                data-testid="voice-provider-open"
                onClick={() =>
                  openVoiceManager("providers")
                }
              >
                {t("tts.providers.dropdown.open")}
              </Button>
            </div>
          )}
          {selectedVoiceNotSetUp && (
            <div
              role="status"
              data-testid="voice-setup-hint"
              className="mt-1 text-xs text-amber-700 dark:text-amber-400"
            >
              <p>
                {noVoiceSetUp
                  ? t("tts.voices.noEngineHint")
                  : t("tts.voices.notSetUpHint")}
              </p>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                data-testid="voice-setup-button"
                onClick={() => openModelsArea("voices")}
              >
                {t("tts.voices.setUp")}
              </Button>
            </div>
          )}
          {piperReaderVoice && (
            <p
              className="mt-1 flex flex-wrap items-center gap-x-2 text-xs text-text/70"
              data-testid="piper-reader-hint"
            >
              <span>
                {t("tts.voices.autoLanguageReads", {
                  name: piperVoiceLabel(piperReaderVoice),
                })}
              </span>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() =>
                  void updateSetting("tts_piper_auto_language", false)
                }
                data-testid="piper-auto-language-off"
              >
                {t("tts.voices.autoLanguageOff")}
              </Button>
            </p>
          )}
          <p
            id="voice-mode-hint"
            className="sr-only"
            data-testid="voice-mode-hint"
          >
            {scriptMode
              ? t("tts.voices.scriptVoicesHint")
              : t("tts.voices.singleVoiceHint")}
          </p>
        </div>
      </TooltipTrigger>
  );
  const readAloudActions = (compact: boolean) => (
      <ReadAloudActions
        compact={compact}
        tab={tab}
        hasText={text.trim().length > 0}
        hasSpokenText={spokenText.trim().length > 0}
        onLoadDocument={() => void loadDocument()}
        onLoadUrl={() => setUrlDialogOpen(true)}
        onAddProjectFile={() => void addFileToProject()}
        dictating={dictating}
        onToggleDictation={toggleDictation}
        saving={saving}
        onSave={saveSpokenAudio}
        exportProgress={exportProgress}
        exportEta={exportEta}
        formatEta={formatEta}
        onCancelExport={cancelExport}
        prewarm={prewarm}
        onPrewarm={() => void startPrewarm()}
        onCancelPrewarm={() => void commands.ttsPrewarmCancel()}
        sentenceCounter={sentenceCounter}
        onOpenWorkshop={() => setWorkshopOpen(true)}
        tidying={tidying}
        onTidy={() => void tidyText()}
        errorCount={errorCount}
        onCheckScript={runScriptCheck}
        autoTagBlockedReason={
          pausesOnly
            ? t("tts.autotag.noStyleTags", {
                engine: engineCaps.providerLabel ?? "",
              })
            : undefined
        }
        autoTag={{
          text,
          suggestions: tagSuggestions,
          sourceText: tagSuggestionsSourceText,
          onSuggestionsChange: changeTagSuggestions,
          onApplyText: applyAutoTagText,
          options:
            autoTagOptions ??
            (getSetting("tts_autotag_last") as AutoTagOptions | null) ??
            defaultAutoTagOptions(),
          onOptionsChange: setAutoTagOptions,
          uiLang,
        }}
        targetLang={targetLang}
        onTargetLangChange={(value) =>
          void updateSetting("tts_translate_lang", value)
        }
        translating={translating}
        onTranslate={translateText}
        summarizing={summarizing}
        onSummarize={summarize}
        transformProgress={transformProgress}
        onCancelTransform={(kind) =>
          void commands.ttsTransformCancel(kind)
        }
        onSwapSpeakers={() => void openSpeakerSwap()}
        sumLength={sumLength}
        onSumLengthChange={setSumLength}
        sumDetail={sumDetail}
        onSumDetailChange={setSumDetail}
        sumAudience={sumAudience}
        onSumAudienceChange={setSumAudience}
      />
  );

  return (
    <PageShell
      title={t("tts.title")}
      /* Ohne Untertitel (Patrick 01.11.): der Weg steht in der Hilfe. */
      fill
      actions={
        <>
          {/* Rueckgaengig / Wiederherstellen wie in Word: oben, nur Pfeile.
              Die Reiterzeile ist in der schmalen Editorspalte schon voll. */}
          <HistoryButtons
            past={history.past}
            future={history.future}
            present={history.present}
            onUndo={history.undo}
            onRedo={history.redo}
            onJump={history.jump}
          />
          <span className="mx-1 h-5 w-px bg-mid-gray/30" aria-hidden="true" />
          <IconButton
            onClick={() => {
              setHelpOpenValue(helpOpen ? "0" : "1");
            }}
            label={t("help.open")}
            icon={<HelpCircle width={20} height={20} aria-hidden="true" />}
          />
        </>
      }
    >
      {/* Volle Hoehe: Seiten links, Text in der Mitte, Bedienung rechts vom
        Text, Dateien/Hilfe ganz rechts. Nur die Spalten scrollen, der Kopf
        und der Rahmen stehen (Entscheidung Patrick 14.09. abends). */}
      <div
        ref={ws.ref}
        className="tts-workspace tts-workspace--fill w-full flex gap-4 items-stretch"
      >
        <PagesSidebar
          pages={pages}
          activeId={activePage}
          collapsed={pagesCollapsed === "1"}
          onToggle={() => setPagesCollapsed(pagesCollapsed === "1" ? "0" : "1")}
          onSelect={setActivePage}
          onChanged={() => void reloadPages()}
          onReplaceSpeaker={() => setReplaceSpeaker({})}
          width={ws.pages.width}
          contents={
            viewMode === "read" ? (
              <ReaderContents
                entries={contents}
                current={chapterIndex}
                onGo={goToChapter}
                pages={contentsPages}
                book={book}
                onIsbn={
                  tab === "original" && activePage
                    ? () => setIsbnOpen(true)
                    : undefined
                }
              />
            ) : null
          }
        />
        {pagesCollapsed !== "1" && (
          <ResizeHandle
            testId="resize-pages"
            label={t("tts.layout.resizePages")}
            direction={1}
            value={ws.pages.width}
            min={ws.pages.min}
            max={ws.pages.max}
            defaultValue={ws.pages.def}
            onChange={ws.pages.set}
          />
        )}
        <div className="flex-1 min-w-0 min-h-0 flex gap-4">
          <div className="tts-editor flex-1 min-w-0 min-h-0 flex flex-col rounded-lg border border-mid-gray/20 bg-background overflow-hidden">
            {truncated && (
              <p className="px-4 pb-2 text-sm text-orange-400">
                {t("tts.truncatedWarning", {
                  limit: truncated.limit,
                  total: truncated.total,
                })}
              </p>
            )}
            {showVramHint && (
              <p className="px-4 pb-2 text-sm text-text/70">
                {t("tts.vramHint")}
              </p>
            )}
            {lastError && (
              <p className="px-4 pb-2 text-sm text-red-500 break-words">
                {lastError}
              </p>
            )}
            {/* overflow-y-auto: reicht der Platz selbst dann nicht (kleines
              Fenster, Korrekturpanel offen), scrollt die Spalte, statt dass
              der Editor unter seine Mindesthoehe gedrueckt wird. */}
            <div
              ref={readerFrameRef}
              className="reader-frame px-4 pb-3 flex-1 min-h-0 flex flex-col gap-2 overflow-y-auto"
              data-reader-theme={viewMode === "read" ? readerTheme : undefined}
              data-fullscreen={fullscreen ? "1" : undefined}
              style={
                awakeShift
                  ? { position: "relative", left: awakeShift, top: awakeShift }
                  : undefined
              }
            >
              {/* Zwei Reiter, ein Feld. Das Original wird nie ueberschrieben —
              die Uebersetzung liegt daneben, nicht darin. Wer zurueckschaltet,
              findet seinen Text unveraendert vor. */}
              <div className="flex flex-wrap items-end gap-x-3 gap-y-1 border-b border-mid-gray/20">
                <TabList
                  tabs={[
                    { id: "original", label: t("tts.tabOriginal") },
                    { id: "translation", label: t("tts.tabTranslation") },
                    { id: "summary", label: t("tts.tabSummary") },
                  ]}
                  value={tab}
                  onChange={setTab}
                  ariaLabel={t("tts.tabsLabel")}
                  className="min-w-0 flex-auto"
                />
                {/* Eigene Breite (flex-auto statt flex-1): reicht der Platz
                    nicht, bricht die Leiste um, statt über den Reitern zu liegen. */}
                <div className="ml-auto max-w-full pb-1">
                  <ReaderToolbar
                    mode={viewMode}
                    canRead={spokenText.trim().length > 0}
                    onMode={chooseViewMode}
                    twoPages={twoPagesStored === "1"}
                    twoPagesTooNarrow={
                      twoPagesStored === "1" &&
                      viewMode === "read" &&
                      pageInfo !== null &&
                      pageInfo.perView === 1
                    }
                    onTwoPages={(two) => setTwoPages(two ? "1" : "0")}
                    fontSize={fontStored}
                    onFontSize={setFontSize}
                    showSpeakers={speakersShown === "1"}
                    onShowSpeakers={(show) => setSpeakersShown(show ? "1" : "0")}
                    follow={follow}
                    onFollow={setFollow}
                    fontFamily={fontFamily}
                    onFontFamily={setFontFamily}
                    theme={readerTheme}
                    onTheme={setReaderTheme}
                    keepAwake={keepAwake}
                    onKeepAwake={toggleKeepAwake}
                    fullscreen={fullscreen}
                    onFullscreen={toggleFullscreen}
                    info={
                      <InfoPopover
                        label={t("tts.writingRules.title")}
                        testId="reader-info"
                      >
                        <p className="mb-1 font-semibold">
                          {t("tts.writingRules.title")}
                        </p>
                <div className="space-y-1.5 text-sm text-text/80">
                  <p>{t("tts.dialogHint")}</p>
                  {pausesOnly ? (
                    <p>
                      {t("tts.writingRules.pausesOnly", {
                        engine: engineCaps.providerLabel ?? "",
                      })}
                    </p>
                  ) : (
                    <p>{t("tts.writingRules.tagsIntro")}</p>
                  )}
                  <ul className="list-disc space-y-0.5 ps-4">
                    {!pausesOnly && (
                      <li>
                        <code className="rounded bg-logo-primary/15 px-1 text-text/70">
                          {t("tts.writingRules.example1")}
                        </code>
                      </li>
                    )}
                    <li>
                      <code className="rounded bg-logo-primary/15 px-1 text-text/70">
                        {t("tts.writingRules.example2")}
                      </code>
                    </li>
                    {!pausesOnly && (
                      <li>
                        <code className="rounded bg-logo-primary/15 px-1 text-text/70">
                          {t("tts.writingRules.example3")}
                        </code>
                      </li>
                    )}
                  </ul>
                </div>
                      </InfoPopover>
                    }
                    bookmarks={
                      <ReaderBookmarks
                        bookmarks={bookmarks}
                        onAdd={addBookmarkHere}
                        onGo={goToBookmark}
                        onRemove={dropBookmark}
                        pageOf={(o) => readerApiRef.current?.pageOfOffset(o) ?? null}
                      />
                    }
                  />
                </div>
              </div>

              {tab === "translation" && translatedLangs.length > 0 && (
                <div
                  role="tablist"
                  aria-label={t("tts.langTabs")}
                  className="flex flex-wrap gap-1"
                  data-testid="translation-lang-tabs"
                >
                  {translatedLangs.map((lang) => {
                    const label =
                      TTS_TARGET_LANGS.find((l) => l.value === lang)?.label ??
                      lang;
                    const active = lang === targetLang;
                    return (
                      <button
                        key={lang}
                        type="button"
                        role="tab"
                        aria-selected={active}
                        onClick={() =>
                          void updateSetting("tts_translate_lang", lang)
                        }
                        data-testid={`lang-tab-${lang}`}
                        className={`cursor-pointer rounded-full border px-3 py-0.5 text-xs transition-colors ${
                          active
                            ? "border-logo-primary bg-logo-primary/15 text-text"
                            : "border-mid-gray/30 text-text/70 hover:text-text"
                        }`}
                      >
                        {label}
                      </button>
                    );
                  })}
                </div>
              )}

              {/* Der Chip-Editor ist Drop-in für die frühere Textarea: die
                native textarea darin bleibt die einzige Wahrheit, Tags
                (`[…]`) erscheinen als Chips im Mirror-Overlay. */}
              {/* Skript-Pruefung: das Korrekturpanel nur nach Klick auf
                "Skript pruefen" -- die Unterstreichung im Text ist davon
                unabhaengig und laeuft (bei Automatik) immer mit. */}
              {panelOpen && scriptCheckOn && spokenText.trim().length > 0 && (
                <div className="pb-2">
                  <ScriptCheckPanel
                    findings={scriptFindings}
                    speakers={speakers}
                    engine={scriptEngine}
                    uiLang={uiLang}
                    actions={scriptActions}
                    onClose={() => setPanelOpen(false)}
                  />
                </div>
              )}
              {viewMode === "read" ? (
                <ReaderView
                  text={spokenText}
                  speakers={speakers}
                  lang={tab === "translation" ? targetLangCode(targetLang) : uiLang}
                  current={readRange}
                  clock={sessionTab.current === tab ? sentenceClock : null}
                  marks={readerMarks}
                  twoPages={twoPagesStored === "1"}
                  fontSize={Number(fontStored)}
                  fontFamily={fontFamily}
                  showSpeakers={speakersShown === "1"}
                  follow={follow}
                  onFollowChange={setFollow}
                  onActivate={(offset) => {
                    setFollow(true);
                    void speakFrom(offset, false);
                  }}
                  onSpeakerMenu={(name) => setReplaceSpeaker({ from: name })}
                  onPageInfo={setPageInfo}
                  apiRef={readerApiRef}
                  title={pages.find((p) => p.id === activePage)?.title}
                />
              ) : (
              <div className="tts-editor__fill flex-1 min-h-[160px]">
                {tab === "original" ? (
                  <TtsChipEditor
                    value={text}
                    onChange={setText}
                    providers={chipProviders}
                    insertApiRef={editorApiRef}
                    pausesOnly={pausesOnly}
                    engineLabel={engineCaps.providerLabel ?? undefined}
                    placeholder={t("tts.inputPlaceholder")}
                    className="w-full"
                    suggestions={tagSuggestions}
                    onResolveSuggestion={resolveTagSuggestion}
                    findings={editorFindings}
                    onUndo={history.undo}
                    onRedo={history.redo}
                    onSpeakFrom={(offset, onlyOne) =>
                      void speakFrom(offset, onlyOne)
                    }
                    onReplaceSpeakerEverywhere={(name) =>
                      setReplaceSpeaker({ from: name })
                    }
                  />
                ) : tab === "translation" ? (
                  <TtsChipEditor
                    value={translation ?? ""}
                    onChange={setTranslation}
                    providers={chipProviders}
                    insertApiRef={editorApiRef}
                    pausesOnly={pausesOnly}
                    engineLabel={engineCaps.providerLabel ?? undefined}
                    placeholder={t("tts.translationPlaceholder")}
                    className="w-full"
                    lang={targetLangCode(targetLang)}
                    findings={editorFindings}
                    onUndo={history.undo}
                    onRedo={history.redo}
                    onSpeakFrom={(offset, onlyOne) =>
                      void speakFrom(offset, onlyOne)
                    }
                    onReplaceSpeakerEverywhere={(name) =>
                      setReplaceSpeaker({ from: name })
                    }
                  />
                ) : (
                  <TtsChipEditor
                    value={summary}
                    onChange={setSummary}
                    providers={chipProviders}
                    insertApiRef={editorApiRef}
                    pausesOnly={pausesOnly}
                    engineLabel={engineCaps.providerLabel ?? undefined}
                    placeholder={t("tts.summaryPlaceholder")}
                    className="w-full"
                    findings={editorFindings}
                    onUndo={history.undo}
                    onRedo={history.redo}
                    onSpeakFrom={(offset, onlyOne) =>
                      void speakFrom(offset, onlyOne)
                    }
                    onReplaceSpeakerEverywhere={(name) =>
                      setReplaceSpeaker({ from: name })
                    }
                  />
                )}
              </div>
              )}
              <ReaderPlayerBar
                speaking={speaking}
                canPlay={spokenText.trim().length > 0}
                canSeek={canResume || speaking}
                onPlayPause={speaking ? pauseSpeaking : () => void speak()}
                onSeek={(delta) => {
                  setFollow(true);
                  seekSentence(delta);
                }}
                onStop={stopSpeaking}
                speed={getSetting("tts_speed") ?? 1.0}
                speeds={SPEEDS}
                speedLabel={speedLabel}
                onSpeed={(value) => void updateSetting("tts_speed", value)}
                progress={
                  speakProgress && sessionTab.current === tab
                    ? {
                        position: currentIndex ?? speakProgress.position,
                        total: speakProgress.total,
                      }
                    : null
                }
                estimateSeconds={readEstimate}
                clock={sessionTab.current === tab ? sentenceClock : null}
                page={viewMode === "read" ? pageInfo : null}
                onJumpToPage={(n) => {
                  setFollow(false);
                  readerApiRef.current?.goToPage(n);
                }}
                onSeekFraction={(fraction) => {
                  // Sprung an die Stelle im Text: ab dem Satz dort vorlesen.
                  const offset = Math.round(fraction * spokenText.length);
                  setFollow(true);
                  void speakFrom(offset, false);
                }}
                onSleep={pauseSpeaking}
                resumable={resumeAt !== null && !canResume && !speaking}
                resumePage={resumePage}
                chapter={contents[chapterIndex]?.title ?? null}
                dock={
                  rightHidden
                    ? (inPopover) => (
                        <>
                          <div
                            className={
                              inPopover ? "w-full" : "reader-dock__voice"
                            }
                          >
                            {voicePickerOf(!inPopover)}
                          </div>
                          {/* In der Zeile sind die Hinweise unter der Stimme
                            verborgen: fehlt eine eingerichtete Stimme, steht
                            der Weg dorthin als Warnknopf daneben. */}
                          {!inPopover && selectedVoiceNotSetUp && (
                            <button
                              type="button"
                              className="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-md text-amber-700 hover:bg-mid-gray/15 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary dark:text-amber-400"
                              aria-label={t("tts.voices.setUp")}
                              title={
                                noVoiceSetUp
                                  ? t("tts.voices.noEngineHint")
                                  : t("tts.voices.notSetUpHint")
                              }
                              onClick={() => openModelsArea("voices")}
                              data-testid="reader-dock-voice-setup"
                            >
                              <TriangleAlert
                                width={16}
                                height={16}
                                aria-hidden="true"
                              />
                            </button>
                          )}
                          {readAloudActions(!inPopover)}
                        </>
                      )
                    : undefined
                }
                onRestart={() => {
                  forgetResumePoint(resumeKeyRef.current);
                  readerApiRef.current?.goToPage(0);
                }}
              />
            </div>
          </div>
          {/* Bedienung rechts vom Text: Transport, Tempo, Stimme, Speichern,
            aktueller Satz, Ausdruck & Sprechstil, Schreibregeln. Scrollt fuer
            sich, wenn die Klappen offen sind. */}
          {/* Rechte Spalte: gestapelt sind Bedienung und Dateien EINE Spalte
            (Griff davor, Breite ws.right); nebeneinander loest sich der
            Rahmen auf (display: contents) und die beiden stehen wie frueher
            als Geschwister neben dem Editor, der Griff sitzt vor den Dateien. */}
          {!rightHidden && (
            <>
          {ws.stacked && (
            <ResizeHandle
              testId="resize-right"
              label={t("tts.layout.resizeRight")}
              direction={-1}
              value={ws.right.width}
              min={ws.right.min}
              max={ws.right.max}
              defaultValue={ws.right.def}
              onChange={ws.right.set}
            />
          )}
          <div
            data-testid="tts-right-column"
            className={ws.stacked ? "tts-right-column--stacked" : "contents"}
            style={
              ws.stacked
                ? ({ "--tts-w": `${ws.right.width}px` } as React.CSSProperties)
                : undefined
            }
          >
            <aside
              data-testid="tts-controls"
              className={`tts-controls shrink-0 min-h-0 overflow-y-auto space-y-3 pe-1 ${
                ws.stacked ? "tts-controls--stacked" : "w-72"
              }`}
              aria-label={t("tts.controls")}
            >
              <div className="flex gap-1 items-start">
                {/* Transport per design system: round glyph buttons, exactly one
                primary. Reading aloud is playback, so it gets the same family
                as every audio player in the app — not text buttons. */}
                {/* Vollstaendige Transportzeile nach Katalog: von der Mitte nach
                aussen — Hauptschalter, daneben die Satzspruenge; hinter dem
                Trenner die Aktionen, die die Wiedergabe nicht fortbewegen.
                Statt ±15 s stehen hier Saetze: vorgelesener Text ist satzweise
                aufgebaut, eine Sekundenmarke gibt es darin nicht. */}
                {/* Transport, Tempo und Fortschritt stehen in der Spielerleiste
                  unter dem Text (lva#148, Codex: Fortschritt und Transport
                  zusammen, getrennt vom Bearbeiten). */}
                {/* Die Stimme dort, wo man sie wechselt: beim Hoeren. Wechsel
                  wirkt sofort — eine laufende Wiedergabe stellt am aktuellen
                  Satz um. Leerer Wert = Standardstimme (Seed). Verwaltung
                  (aufnehmen, importieren, loeschen) unten bei den
                  Einstellungen. */}
                {/* Die Wahl in Worten: mit Sprechern oder alles in einer
                  Stimme. Kein Dauertext mehr in der schmalen Spalte, sondern
                  Tooltip beim Ueberfahren/Fokussieren und Beschreibung fuer
                  Screenreader (aria-describedby) -- sonst wundert man sich,
                  warum die Marker nicht mehr umschalten. */}
                <div className="min-w-0 flex-1">{voicePickerOf()}</div>
                {/* Ganze Spalte ausblenden (Variante A): Stimme und Werkzeuge
                  ziehen dann in die Spielerleiste unter dem Text. */}
                <button
                  type="button"
                  className="flex h-9 w-8 shrink-0 cursor-pointer items-center justify-center rounded-md text-text/70 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
                  aria-label={t("tts.layout.hideColumn")}
                  title={t("tts.layout.hideColumn")}
                  onClick={() => setRightHidden("1")}
                  data-testid="right-column-hide"
                >
                  <PanelRightClose width={16} height={16} aria-hidden="true" />
                </button>
              </div>
              {readAloudActions(false)}
              {speaking && currentSentence && viewMode === "edit" && (
                <p className="text-sm italic text-text/70 border-s-2 border-logo-primary ps-2">
                  {currentSentence}
                </p>
              )}
            </aside>
            {!ws.stacked && filesCollapsed !== "1" && (
              <ResizeHandle
                testId="resize-right"
                label={t("tts.layout.resizeFiles")}
                direction={-1}
                value={ws.right.width}
                min={ws.right.min}
                max={ws.right.max}
                defaultValue={ws.right.def}
                onChange={ws.right.set}
              />
            )}
            {helpOpen && !rightHidden && (
              <TtsHelpDock onClose={() => setHelpOpenValue("0")} />
            )}
            <FilesSidebar
              pageId={activePage}
              collapsed={filesCollapsed === "1"}
              onToggle={() =>
                setFilesCollapsed(filesCollapsed === "1" ? "0" : "1")
              }
              width={ws.right.width}
              stacked={ws.stacked}
              onLayoutToggle={ws.toggleLayout}
              /* Der Text einer erzeugten Aufnahme zurueck in den Editor: die eine
             falsche Zeile aendern und erneut erzeugen. Die unveraenderten
             Saetze kommen dann aus dem Satz-Cache, nur die geaenderten gehen
             durch die Engine — ein Hoerspiel muss dafuer nicht neu entstehen. */
              onUseText={(value) => {
                setTab("original");
                setText(value);
              }}
            />
          </div>
            </>
          )}

          {/* Hilfe bleibt auch bei ausgeblendeter Spalte erreichbar. */}
          {helpOpen && rightHidden && (
            <TtsHelpDock onClose={() => setHelpOpenValue("0")} />
          )}
          {/* Ausgeblendete rechte Spalte: derselbe Streifen wie links. */}
          {rightHidden && (
            <ColumnStrip
              side="right"
              label={t("tts.layout.showColumn")}
              onClick={() => setRightHidden("0")}
              testId="right-column-show"
            />
          )}

          <Dialog
            open={checkDialog !== null}
            onOpenChange={(isOpen) => {
              if (!isOpen) setCheckDialog(null);
            }}
            title={t("tts.scriptCheck.dialogTitle", {
              count: scriptFindings.length,
            })}
            closeLabel={t("tts.scriptCheck.dialogCancel")}
            footer={
              <>
                <Button
                  variant="secondary"
                  onClick={() => setCheckDialog(null)}
                >
                  {t("tts.scriptCheck.dialogCancel")}
                </Button>
                <Button
                  variant="primary"
                  data-testid="script-check-proceed"
                  onClick={() => {
                    const action = checkDialog;
                    setCheckDialog(null);
                    if (action === "speak") void speakNow();
                    if (action === "save") void saveSpokenAudioNow();
                  }}
                >
                  {checkDialog === "save"
                    ? t("tts.scriptCheck.dialogSaveAnyway")
                    : t("tts.scriptCheck.dialogSpeakAnyway")}
                </Button>
              </>
            }
          >
            <p className="text-sm text-text/80">
              {t("tts.scriptCheck.dialogBody", {
                count: scriptFindings.length,
              })}
            </p>
          </Dialog>

          <ReplaceSpeakerDialog
            open={replaceSpeaker !== null}
            onOpenChange={(isOpen) => {
              if (!isOpen) setReplaceSpeaker(null);
            }}
            speakers={speakers}
            providerId={providerEngineId}
            pages={pages}
            activePageId={activePage}
            active={{ text, summary, voices: tabVoices }}
            onApplyActive={(next) => {
              setText(next.text);
              setSummary(next.summary);
              setTabVoices(next.voices);
            }}
            onSaved={() => void reloadPages()}
            initialFrom={replaceSpeaker?.from}
          />
          {voiceMenu.menu}
          <IsbnDialog
            open={isbnOpen}
            onOpenChange={setIsbnOpen}
            initialIsbn={isbnGuess}
            onApply={applyIsbnBook}
          />
          <SpeakerSwapDialog
            open={swapOpen}
            onOpenChange={setSwapOpen}
            sources={speakersInText(translation ?? "", langVoices)}
            voices={langVoices}
            targetLang={targetLang}
            targetCode={targetLangCode(targetLang)}
            initial={swapInitial}
            onApply={applySpeakerSwap}
          />
          <ScriptWorkshopDialog
            open={workshopOpen}
            onOpenChange={setWorkshopOpen}
            pages={pages}
            activePageId={activePage}
            onCreatePage={createPageWithText}
            onPagesChanged={() => void reloadPages()}
          />
          <Dialog
            open={urlDialogOpen}
            onOpenChange={setUrlDialogOpen}
            title={t("tts.add.urlDialogTitle")}
            closeLabel={t("tts.stopConfirmCancel")}
            footer={
              <>
                <Button
                  variant="secondary"
                  onClick={() => setUrlDialogOpen(false)}
                >
                  {t("tts.stopConfirmCancel")}
                </Button>
                <Button
                  onClick={() => {
                    setUrlDialogOpen(false);
                    void loadUrl();
                  }}
                  disabled={loadingSource || sourceUrl.trim().length === 0}
                >
                  {t("tts.summary.loadUrl")}
                </Button>
              </>
            }
          >
            <Input
              type="text"
              value={sourceUrl}
              onChange={(e) => setSourceUrl(e.target.value)}
              placeholder={t("tts.summary.urlPlaceholder")}
              className="w-full"
              autoFocus
              onKeyDown={(e) => {
                if (e.key === "Enter" && sourceUrl.trim()) {
                  setUrlDialogOpen(false);
                  void loadUrl();
                }
              }}
            />
          </Dialog>
        </div>
      </div>
    </PageShell>
  );
};
