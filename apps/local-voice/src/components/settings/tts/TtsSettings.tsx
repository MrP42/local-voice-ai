import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { commands, type PageInfo, type TtsStatus } from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { ShortcutInput } from "../ShortcutInput";
import { VoicesCard } from "./VoicesCard";
import { FilesSidebar, PagesSidebar } from "./WorkspaceSidebars";
import { VoiceChangerCard } from "./VoiceChangerCard";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { Input } from "../../ui/Input";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { Slider } from "../../ui/Slider";
import { Select } from "../../ui/Select";
import { ReadingCard } from "./ReadingCard";
import {
  TtsChipEditor,
  type ChipEditorInsertApi,
  type ChipEditorSuggestion,
} from "./editor/TtsChipEditor";
import { useTagProvider } from "./tags/tagProvider";
import { TagPalette } from "./tags";
import { AutoTagBar, resolveSuggestion } from "./tags/AutoTagBar";
import { usePersistentState } from "../../../hooks/usePersistentState";
import {
  TTS_TARGET_LANGS,
  targetLangCode,
} from "../../../lib/constants/languages";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Glyph } from "../../ui/AudioPlayer";
import {
  BrainCircuit,
  Dices,
  Download,
  FilePlus2,
  FileText,
  Languages,
  Link,
  Mic,
  Plus,
  Save,
  Server,
  Upload,
} from "lucide-react";

/// Abspieltempo der Transportleiste. Bewusst grob gestuft: feiner regelt der
/// Schieber in den Einstellungen, hier will man im Hoeren einmal schneller
/// oder langsamer stellen, nicht justieren.
const SPEEDS = [0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

export const TtsSettings = () => {
  const { t, i18n } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const uiLang = i18n.language?.split("-")[0] ?? "en";
  /** Tag-Chips in allen drei Text-Reitern; der Sprecher-Provider eines
   *  späteren Pakets kommt einfach mit in dieses Array. */
  const tagProvider = useTagProvider();
  const chipProviders = useMemo(() => [tagProvider], [tagProvider]);
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
  /** T4 Auto-Tagging: offene Vorschläge im Original-Reiter (gestrichelte
   *  Chips im Editor). Gehört der Seite hier, weil sowohl der Editor
   *  (Popover-Buttons) als auch AutoTagBar ("Alle annehmen/verwerfen")
   *  dieselbe Liste + denselben Text splicen müssen. */
  const [tagSuggestions, setTagSuggestions] = useState<ChipEditorSuggestion[]>(
    [],
  );
  const [pages, setPages] = useState<PageInfo[]>([]);
  const [activePage, setActivePage] = usePersistentState<string>(
    "tts.activePage",
    "",
  );
  // Beide Seitenspalten starten eingeklappt: der Lesetext ist die Arbeit,
  // die Spalten sind Navigation. Wer sie aufklappt, behaelt das (persistent).
  // ".v2"-Schluessel: der alte Schluessel hat bei jedem Bestandsnutzer "0"
  // persistiert — ohne Umbenennung saehe niemand den neuen Default.
  const [pagesCollapsed, setPagesCollapsed] = usePersistentState<string>(
    "tts.pagesCollapsed.v2",
    "1",
  );
  const [filesCollapsed, setFilesCollapsed] = usePersistentState<string>(
    "tts.filesCollapsed.v2",
    "1",
  );
  /** Erst nach dem Laden einer Seite darf gespeichert werden — sonst
   *  ueberschriebe der leere Anfangszustand den echten. */
  const pageLoaded = useRef(false);
  /** Zu welchem Reiter die laufende Sprechsitzung gehoert. Fortsetzen und
   *  Satzspruenge gelten nur fuer sie — ein anderer Reiter startet neu. */
  const sessionTab = useRef<"original" | "translation" | "summary">("original");
  /** Zielsprache der Uebersetzung — dieselbe Einstellung wie in der
   *  Audio-Uebersetzung, damit man sie nicht an zwei Stellen pflegt. */
  const targetLang = getSetting("tts_translate_lang") ?? "English";
  /** Alle Referenzstimmen — fuer das Dropdown an der Transportleiste. */
  const [voices, setVoices] = useState<string[]>([]);
  /** Dialog: die aktuelle Seed-Stimme unter einem Namen sichern. */
  const [saveSeedOpen, setSaveSeedOpen] = useState(false);
  const [seedName, setSeedName] = useState("");
  const [savingSeed, setSavingSeed] = useState(false);
  /** Zusammenfassung des Originals — dritter Reiter. */
  const [summary, setSummary] = useState<string>("");
  const [summarizing, setSummarizing] = useState(false);
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
  /** Plus-Menue fuer Quellen (Dokument, Webseite, Projektdatei). */
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  const [urlDialogOpen, setUrlDialogOpen] = useState(false);
  /** Welcher Reiter offen ist. Das Original bleibt immer erhalten. */
  const [tab, setTab] = useState<"original" | "translation" | "summary">(
    "original",
  );
  const [translation, setTranslation] = useState<string | null>(null);
  const [translating, setTranslating] = useState(false);
  const [dictating, setDictating] = useState(false);
  const [startingSeconds, setStartingSeconds] = useState(0);
  const [lastError, setLastError] = useState<string | null>(null);
  /** Rueckmeldung des harten Beendens — was gefunden und beendet wurde. */
  const [killNotice, setKillNotice] = useState<string | null>(null);
  /** Der Vorlesetext wurde an der Zeichengrenze gekappt. */
  const [truncated, setTruncated] = useState<{
    limit: number;
    total: number;
  } | null>(null);
  /** Sprachmodell-Anzeige: was Ollama geladen hat, ob gerade uebersetzt
   *  wird, und der letzte Fehler. */
  const [llmLoaded, setLlmLoaded] = useState<string[]>([]);
  const [llmBusy, setLlmBusy] = useState(false);
  const [llmError, setLlmError] = useState<string | null>(null);
  const [llmDialog, setLlmDialog] = useState(false);
  const [llmWorking, setLlmWorking] = useState(false);
  /** Offene Rueckfrage vor dem Beenden des Servers. */
  const [confirmStop, setConfirmStop] = useState(false);
  const [speakProgress, setSpeakProgress] = useState<{
    position: number;
    total: number;
  } | null>(null);
  const [currentSentence, setCurrentSentence] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [exportProgress, setExportProgress] = useState<{
    position: number;
    total: number;
  } | null>(null);
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
        // Fertige Datei: die Dateileiste rechts soll sie sofort zeigen.
        window.dispatchEvent(new CustomEvent("lv-files-changed"));
        return;
      }
      setExportProgress({ position, total });
    });
    const unExportError = listen<{ message: string }>(
      "tts-export-error",
      (e) => {
        setSaving(false);
        setExportProgress(null);
        setLastError(e.payload.message);
      },
    );
    const unProgress = listen<{ position: number; total: number }>(
      "tts-speak-progress",
      (e) => setSpeakProgress(e.payload),
    );
    const unSentence = listen<{ context: string; text: string }>(
      "tts-current-sentence",
      (e) => {
        if (e.payload.context === "speak") setCurrentSentence(e.payload.text);
      },
    );
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
      if (!abandoned) setTranslation(hit);
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
          };
          setText(state.text ?? "");
          setSummary(state.summary ?? "");
          setSourceUrl(state.sourceUrl ?? "");
          setTab(
            state.tab === "translation" || state.tab === "summary"
              ? state.tab
              : "original",
          );
        } catch {
          setText(result.data);
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
        setText(legacy("tts.text"));
        setSummary(legacy("tts.summary"));
        setSourceUrl(legacy("tts.summary.url"));
        setTab("original");
      }
      setTranslation(null);
      pageLoaded.current = true;
    });
  }, [activePage]);

  // Arbeitsstand sichern — gebuendelt, eine halbe Sekunde nach der letzten
  // Aenderung. Jeder Tastendruck einzeln waere ein Schreibzugriff zu viel.
  useEffect(() => {
    if (!activePage || !pageLoaded.current) return;
    const handle = window.setTimeout(() => {
      void commands.pageStateSave(
        activePage,
        JSON.stringify({ text, summary, sourceUrl, tab }),
      );
    }, 500);
    return () => window.clearTimeout(handle);
  }, [activePage, text, summary, sourceUrl, tab]);

  // Sprachmodell-Anzeige: Ereignis waehrend der Uebersetzung, dazu eine
  // Abfrage alle zehn Sekunden — billig (lokaler Aufruf mit kurzem Timeout)
  // und noetig, weil auch Ollamas eigene Frist ein Modell entlaedt, ohne
  // dass die App davon erfuehre.
  useEffect(() => {
    const poll = () => {
      void commands.llmPs().then(setLlmLoaded);
    };
    poll();
    const timer = window.setInterval(poll, 10_000);
    const un = listen<{ busy: boolean; error?: string | null }>(
      "llm-activity",
      (e) => {
        setLlmBusy(e.payload.busy);
        if (!e.payload.busy) {
          setLlmError(e.payload.error ?? null);
          poll();
        }
      },
    );
    return () => {
      window.clearInterval(timer);
      un.then((f) => f());
    };
  }, []);

  /** Farbe des Sprachmodell-Symbols — dieselbe Sprache wie das Serversymbol:
   *  grau aus, gelb pulsierend arbeitet, gruen geladen, orange Fehler. */
  const llmIconClass = llmBusy
    ? "text-yellow-400 animate-pulse"
    : llmError
      ? "text-orange-500 animate-pulse"
      : llmLoaded.length > 0
        ? "text-green-500"
        : "text-text/40";

  const llmTitle = llmBusy
    ? t("tts.llm.busy")
    : llmError
      ? llmError
      : llmLoaded.length > 0
        ? t("tts.llm.loaded", { models: llmLoaded.join(", ") })
        : t("tts.llm.idle");

  const llmUnloadNow = async () => {
    setLlmWorking(true);
    setLlmError(null);
    const result = await commands.llmUnload();
    setLlmWorking(false);
    setLlmDialog(false);
    if (result.status === "error") setLlmError(result.error);
    setLlmLoaded(await commands.llmPs());
  };

  const llmWarmNow = async () => {
    setLlmWorking(true);
    setLlmError(null);
    setLlmDialog(false);
    setLlmBusy(true);
    const result = await commands.llmWarm();
    setLlmBusy(false);
    setLlmWorking(false);
    if (result.status === "error") setLlmError(result.error);
    setLlmLoaded(await commands.llmPs());
  };

  // Stimmenliste fuer das Dropdown. Die Verwaltung unten meldet
  // Aenderungen ueber ein Fensterereignis, damit beide nie auseinanderlaufen.
  useEffect(() => {
    const load = () => {
      void commands.ttsListVoices().then((r) => {
        if (r.status === "ok") setVoices(r.data);
      });
    };
    load();
    window.addEventListener("lv-voices-changed", load);
    return () => window.removeEventListener("lv-voices-changed", load);
  }, []);

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
    setLastError(null);
    setSpeakProgress(null);
    setTruncated(null);
    sessionTab.current = tab;
    const result = await commands.ttsSpeakText(spokenText);
    if (result.status === "error") setLastError(result.error);
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
    setSpeakProgress(null);
  };

  const resumeSpeaking = async () => {
    setLastError(null);
    const result = await commands.ttsSpeakResume();
    if (result.status === "error") setLastError(result.error);
  };

  /**
   * The whole text — speaker changes and all — written to one WAV instead of
   * only played. Goes through the same segmentation as playback, so the file
   * sounds like what you heard.
   */
  const saveSpokenAudio = async () => {
    setLastError(null);
    // Der Speichern-Dialog schlaegt den Projektordner der Seite vor: dort
    // sammelt die Dateileiste rechts, was zu diesem Arbeitsblatt gehoert.
    // Ein anderer Ort bleibt jederzeit waehlbar.
    let defaultPath = "vorlesen.wav";
    if (activePage) {
      const dir = await commands.pageDir(activePage);
      if (dir.status === "ok") defaultPath = `${dir.data}\\vorlesen.wav`;
    }
    const target = await save({
      filters: [{ name: "WAV", extensions: ["wav"] }],
      defaultPath,
    });
    if (typeof target !== "string") return;
    setSaving(true);
    setExportProgress({ position: 0, total: 0 });
    // Returns at once; the run reports itself through tts-export-progress.
    const result = await commands.ttsSpeakToFile(spokenText, target);
    if (result.status === "error") {
      setSaving(false);
      setExportProgress(null);
      setLastError(result.error);
    }
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

  const canResume =
    !speaking &&
    speakProgress !== null &&
    speakProgress.position < speakProgress.total;

  const startServer = async () => {
    setLastError(null);
    const result = await commands.ttsServerStart();
    if (result.status === "error") setLastError(result.error);
  };

  /**
   * Harter Ausweg: beendet, was auf dem TTS-Port lauscht, ohne vorher zu
   * fragen, ob es antwortet. Meldet zurueck, was gefunden wurde — "nichts
   * gefunden" ist ein Ergebnis und kein Fehler, deshalb steht es als Hinweis
   * und nicht als Fehlermeldung.
   */
  const killServer = async () => {
    setLastError(null);
    const result = await commands.ttsServerKill();
    if (result.status === "error") {
      setLastError(result.error);
      return;
    }
    setKillNotice(result.data);
    window.setTimeout(() => setKillNotice(null), 4000);
  };

  /**
   * Farbe des Serversymbols. Nur Farbe, kein zweites Symbol: die Form soll
   * ueber alle Zustaende gleich bleiben, damit man sie an derselben Stelle
   * wiederfindet — was sich aendert, ist der Zustand, nicht die Sache.
   *
   * Der Fehlerzustand blinkt als einziger. Er ist der einzige, der eine
   * Handlung verlangt, die nicht aufschiebbar ist.
   */
  const serverIconClass =
    phase === "starting"
      ? "text-yellow-400 animate-pulse"
      : phase === "error"
        ? "text-orange-500 animate-pulse"
        : phase === "stopped"
          ? "text-text/40"
          : "text-green-500";

  const serverTitle =
    phase === "stopped"
      ? t("tts.serverIconStart")
      : phase === "starting"
        ? t("tts.serverIconStarting")
        : phase === "error"
          ? (status?.message ?? t("tts.serverIconError"))
          : t("tts.serverIconStop");

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
    const result = await commands.ttsTranslate(text, targetLang);
    setTranslating(false);
    if (result.status === "error") {
      setLastError(result.error);
      return;
    }
    setTranslation(result.data);
    setTab("translation");
  };

  /**
   * T4 Auto-Tagging: Tags wirklich in den Text übernehmen (Annehmen, einzeln
   * oder "Alle annehmen") — mit Undo-Toast, der den Vortext wiederherstellt.
   * Reines Verwerfen ruft das NICHT auf (der Text ändert sich dabei nicht).
   */
  const applyAutoTagText = (
    nextText: string,
    previousText: string,
    count: number,
  ) => {
    setText(nextText);
    toast(t("tts.autotag.appliedToast", { count }), {
      action: {
        label: t("tts.autotag.undo"),
        onClick: () => setText(previousText),
      },
    });
  };

  /** An `TtsChipEditor.onResolveSuggestion` gereicht: Annehmen/Verwerfen
   *  EINES Vorschlags aus dessen Popover (Check/X-Buttons). */
  const resolveTagSuggestion = (id: string, accept: boolean) => {
    const outcome = resolveSuggestion(text, tagSuggestions, id, accept);
    setTagSuggestions(outcome.suggestions);
    if (outcome.inserted) {
      applyAutoTagText(outcome.text, text, 1);
    }
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

  /** Dokument als Originaltext laden (txt/md/pdf/docx). */
  const loadDocument = async () => {
    const picked = await open({
      multiple: false,
      filters: [
        { name: "Dokumente", extensions: ["txt", "md", "pdf", "docx"] },
      ],
    });
    if (typeof picked !== "string") return;
    setLastError(null);
    setLoadingSource(true);
    const result = await commands.ttsExtractDocument(picked);
    setLoadingSource(false);
    if (result.status === "error") {
      setLastError(result.error);
      return;
    }
    setTab("original");
    setText(result.data);
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
   * Zusammenfassen — nur zusammenfassen. Das Ergebnis liegt im dritten
   * Reiter; das Original bleibt unangetastet, abspielen kann man beides.
   */
  const summarize = async () => {
    if (!text.trim()) return;
    setLastError(null);
    setSummarizing(true);
    const result = await commands.ttsSummarizeText(text, {
      length: sumLength,
      detail: sumDetail,
      audience: sumAudience,
    });
    setSummarizing(false);
    if (result.status === "error") {
      setLastError(result.error);
      return;
    }
    setSummary(result.data);
    setTab("summary");
  };

  /**
   * Den aktuellen Seed als benannte Stimme sichern. Ein Seed ist fluechtig —
   * wer weiterwuerfelt, verliert die Stimme, die ihm eben gefiel.
   */
  const saveSeedVoice = async () => {
    if (!seedName.trim()) return;
    setSavingSeed(true);
    setLastError(null);
    const result = await commands.ttsSaveSeedVoice(seedName.trim());
    setSavingSeed(false);
    if (result.status === "error") {
      setLastError(result.error);
      return;
    }
    setSaveSeedOpen(false);
    setSeedName("");
    window.dispatchEvent(new CustomEvent("lv-voices-changed"));
    void updateSetting("tts_voice", result.data);
  };

  /**
   * Ein Klick tut, was im jeweiligen Zustand ansteht. Beim laufenden Server
   * ist das Beenden — und weil damit ein Modellstart von bis zu zwei Minuten
   * verfaellt und laufendes Vorlesen abbricht, wird vorher gefragt.
   */
  const onServerIconClick = () => {
    // IMMER der Dialog, in jeder Phase — wie beim Sprachmodell daneben.
    // Frueher startete ein Klick bei gestopptem Server sofort: ein
    // versehentlicher Klick belegte damit ungefragt 17 GB Grafikspeicher
    // und zwei Minuten Ladezeit. Starten ist eine Entscheidung, keine
    // Beruehrung.
    setConfirmStop(true);
  };

  /**
   * Neu starten = beenden und sofort wieder hochfahren. Der Weg, wenn der
   * Server zwar laeuft, aber nicht mehr vernuenftig antwortet (etwa mit 500);
   * ohne ihn muesste man zweimal klicken und dazwischen raten, wann er
   * wirklich unten ist.
   */
  const restartServer = async () => {
    setLastError(null);
    const killed = await commands.ttsServerKill();
    if (killed.status === "error") {
      setLastError(killed.error);
      return;
    }
    await startServer();
  };

  const showVramHint =
    starting && (startingSeconds >= 120 || status?.message === "vram");

  return (
    <div className="w-full flex gap-4 items-start">
      <PagesSidebar
        pages={pages}
        activeId={activePage}
        collapsed={pagesCollapsed === "1"}
        onToggle={() => setPagesCollapsed(pagesCollapsed === "1" ? "0" : "1")}
        onSelect={setActivePage}
        onChanged={() => void reloadPages()}
      />
      <div className="flex-1 min-w-0 space-y-6">
        <SettingsGroup title={t("tts.title")}>
          <SettingContainer
            title={t("tts.serverTitle")}
            description={t("tts.description")}
            grouped={true}
            layout="horizontal"
          >
            <div className="flex items-center">
              {/* Das Sprachmodell der Nachbearbeitung (Uebersetzen,
                  Zusammenfassen), in derselben Farbsprache wie der Server
                  daneben. Klick: entladen oder vorwaermen. */}
              <button
                type="button"
                onClick={() => setLlmDialog(true)}
                title={llmTitle}
                aria-label={llmTitle}
                className="p-1.5 rounded-md hover:bg-mid-gray/20 transition-colors cursor-pointer"
              >
                <BrainCircuit
                  width={20}
                  height={20}
                  className={llmIconClass}
                  aria-hidden="true"
                />
              </button>
              {/* Ein einziges Element traegt Zustand UND Bedienung. Die Farbe
                sagt, woran man ist — grau (aus), gelb (faehrt hoch), gruen
                (laeuft), orange blinkend (Fehler) —, der Klick tut, was in
                diesem Zustand ansteht. Das Wort daneben war eine zweite
                Anzeige derselben Sache; es steht jetzt im Tooltip, wo es nur
                stoert, wenn man es sucht. */}
              <button
                type="button"
                onClick={onServerIconClick}
                title={serverTitle}
                aria-label={serverTitle}
                className="p-1.5 rounded-md hover:bg-mid-gray/20 transition-colors cursor-pointer"
              >
                <Server
                  width={20}
                  height={20}
                  className={serverIconClass}
                  aria-hidden="true"
                />
              </button>
            </div>
          </SettingContainer>
          {truncated && (
            <p className="px-4 pb-2 text-sm text-orange-400">
              {t("tts.truncatedWarning", {
                limit: truncated.limit,
                total: truncated.total,
              })}
            </p>
          )}
          {killNotice && (
            <p className="px-4 pb-2 text-sm text-text/70">{killNotice}</p>
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
          <div className="px-4 pb-4 space-y-2">
            {/* Zwei Reiter, ein Feld. Das Original wird nie ueberschrieben —
              die Uebersetzung liegt daneben, nicht darin. Wer zurueckschaltet,
              findet seinen Text unveraendert vor. */}
            <div className="flex items-center gap-1 border-b border-mid-gray/20">
              <button
                type="button"
                onClick={() => setTab("original")}
                className={`px-3 py-1.5 text-sm border-b-2 -mb-px transition-colors cursor-pointer ${
                  tab === "original"
                    ? "border-logo-primary text-text"
                    : "border-transparent text-text/50 hover:text-text/80"
                }`}
              >
                {t("tts.tabOriginal")}
              </button>
              <button
                type="button"
                onClick={() => setTab("translation")}
                className={`px-3 py-1.5 text-sm border-b-2 -mb-px transition-colors cursor-pointer ${
                  tab === "translation"
                    ? "border-logo-primary text-text"
                    : "border-transparent text-text/50 hover:text-text/80"
                }`}
              >
                {t("tts.tabTranslation")}
              </button>
              <button
                type="button"
                onClick={() => setTab("summary")}
                className={`px-3 py-1.5 text-sm border-b-2 -mb-px transition-colors cursor-pointer ${
                  tab === "summary"
                    ? "border-logo-primary text-text"
                    : "border-transparent text-text/50 hover:text-text/80"
                }`}
              >
                {t("tts.tabSummary")}
              </button>
            </div>

            {/* Der Chip-Editor ist Drop-in für die frühere Textarea: die
                native textarea darin bleibt die einzige Wahrheit, Tags
                (`[…]`) erscheinen als Chips im Mirror-Overlay. */}
            {tab === "original" ? (
              <TtsChipEditor
                value={text}
                onChange={setText}
                providers={chipProviders}
                insertApiRef={editorApiRef}
                placeholder={t("tts.inputPlaceholder")}
                rows={5}
                className="w-full"
                suggestions={tagSuggestions}
                onResolveSuggestion={resolveTagSuggestion}
              />
            ) : tab === "translation" ? (
              <TtsChipEditor
                value={translation ?? ""}
                onChange={setTranslation}
                providers={chipProviders}
                insertApiRef={editorApiRef}
                placeholder={t("tts.translationPlaceholder")}
                rows={5}
                className="w-full"
                lang={targetLangCode(targetLang)}
              />
            ) : (
              <TtsChipEditor
                value={summary}
                onChange={setSummary}
                providers={chipProviders}
                insertApiRef={editorApiRef}
                placeholder={t("tts.summaryPlaceholder")}
                rows={5}
                className="w-full"
              />
            )}

            <TagPalette
              uiLang={uiLang}
              onInsert={(tagText) =>
                editorApiRef.current?.insertAtCursor(tagText)
              }
              onDragInsert={(x, y, tagText) =>
                editorApiRef.current?.insertAtPoint?.(x, y, tagText) ?? false
              }
            />

            {/* Auto-Tagging (Paket C-T4): nur im Original-Reiter — die
                Vorschläge hängen am dortigen Text und dessen Editor-Chips. */}
            {tab === "original" && (
              <AutoTagBar
                text={text}
                suggestions={tagSuggestions}
                onSuggestionsChange={setTagSuggestions}
                onApplyText={applyAutoTagText}
              />
            )}

            {/* Je Reiter nur die Aktionen, die er braucht — und die Quellen
                gebuendelt hinter EINEM Plus (Dokument, Webseite,
                Projektdatei), wie man es aus KI-Apps kennt. Kein Knopf tut
                zwei Dinge; es steht nur nichts mehr da, was der offene
                Reiter nicht braucht. */}
            <div className="flex items-center gap-2 flex-wrap">
              {tab === "original" && (
                <>
                  <div className="relative">
                    <Button
                      variant="secondary"
                      onClick={() => setAddMenuOpen((o) => !o)}
                      title={t("tts.add.title")}
                      aria-label={t("tts.add.title")}
                      aria-expanded={addMenuOpen}
                    >
                      <Plus width={16} height={16} />
                    </Button>
                    {addMenuOpen && (
                      <>
                        {/* Unsichtbarer Fang fuer den Klick daneben. */}
                        <div
                          className="fixed inset-0 z-30"
                          onClick={() => setAddMenuOpen(false)}
                        />
                        <div className="absolute left-0 top-full mt-1 w-64 rounded-lg border border-mid-gray/40 bg-background shadow-lg z-40 py-1">
                          <button
                            type="button"
                            className="w-full flex items-center gap-2 px-3 py-2 text-sm text-text/80 hover:bg-mid-gray/15 hover:text-text cursor-pointer text-start"
                            onClick={() => {
                              setAddMenuOpen(false);
                              void loadDocument();
                            }}
                          >
                            <Upload width={15} height={15} />
                            {t("tts.add.document")}
                          </button>
                          <button
                            type="button"
                            className="w-full flex items-center gap-2 px-3 py-2 text-sm text-text/80 hover:bg-mid-gray/15 hover:text-text cursor-pointer text-start"
                            onClick={() => {
                              setAddMenuOpen(false);
                              setUrlDialogOpen(true);
                            }}
                          >
                            <Link width={15} height={15} />
                            {t("tts.add.url")}
                          </button>
                          <button
                            type="button"
                            className="w-full flex items-center gap-2 px-3 py-2 text-sm text-text/80 hover:bg-mid-gray/15 hover:text-text cursor-pointer text-start"
                            onClick={() => {
                              setAddMenuOpen(false);
                              void addFileToProject();
                            }}
                          >
                            <FilePlus2 width={15} height={15} />
                            {t("tts.add.projectFile")}
                          </button>
                        </div>
                      </>
                    )}
                  </div>
                  <Button
                    variant="secondary"
                    onClick={toggleDictation}
                    title={
                      dictating ? t("tts.dictateStop") : t("tts.dictateHint")
                    }
                    aria-label={
                      dictating ? t("tts.dictateStop") : t("tts.dictate")
                    }
                  >
                    <Mic
                      width={16}
                      height={16}
                      className={
                        dictating ? "text-red-400 animate-pulse" : undefined
                      }
                    />
                  </Button>
                </>
              )}
              {tab === "translation" && (
                <>
                  <div className="w-36">
                    <Select
                      value={targetLang}
                      options={TTS_TARGET_LANGS}
                      onChange={(value) =>
                        value && updateSetting("tts_translate_lang", value)
                      }
                      isClearable={false}
                    />
                  </div>
                  <Button
                    variant="secondary"
                    onClick={translateText}
                    disabled={translating || !text.trim()}
                    title={
                      translating
                        ? t("tts.translating")
                        : t("tts.translateAction")
                    }
                    aria-label={t("tts.translateAction")}
                  >
                    <Languages width={16} height={16} />
                  </Button>
                </>
              )}
              {tab === "summary" && (
                <Button
                  variant="secondary"
                  onClick={summarize}
                  disabled={summarizing || !text.trim()}
                  title={
                    summarizing ? t("tts.summarizing") : t("tts.summarizeHint")
                  }
                  aria-label={t("tts.summarize")}
                >
                  <FileText width={16} height={16} />
                </Button>
              )}
            </div>

            {/* Wie zusammengefasst wird — wirkt beim naechsten Klick auf
              "Zusammenfassen". Nur im Zusammenfassungs-Reiter sichtbar, wo
              die Frage sich stellt. */}
            {tab === "summary" && (
              <div className="flex gap-3 items-center flex-wrap">
                <label className="flex items-center gap-1 text-sm">
                  {t("tts.summary.length")}
                  <div className="w-40">
                    <Select
                      value={sumLength}
                      isClearable={false}
                      options={[
                        {
                          value: "kurz",
                          label: t("tts.summary.lengths.short"),
                        },
                        {
                          value: "mittel",
                          label: t("tts.summary.lengths.medium"),
                        },
                        { value: "lang", label: t("tts.summary.lengths.long") },
                      ]}
                      onChange={(value) => value && setSumLength(value)}
                    />
                  </div>
                </label>
                <label className="flex items-center gap-1 text-sm">
                  {t("tts.summary.detail")}
                  <div className="w-40">
                    <Select
                      value={sumDetail}
                      isClearable={false}
                      options={[
                        {
                          value: "ueberblick",
                          label: t("tts.summary.details.overview"),
                        },
                        {
                          value: "ausgewogen",
                          label: t("tts.summary.details.balanced"),
                        },
                        {
                          value: "detailliert",
                          label: t("tts.summary.details.deep"),
                        },
                      ]}
                      onChange={(value) => value && setSumDetail(value)}
                    />
                  </div>
                </label>
                <label className="flex items-center gap-1 text-sm">
                  {t("tts.summary.audience")}
                  <div className="w-44">
                    <Select
                      value={sumAudience}
                      isClearable={false}
                      options={[
                        {
                          value: "allgemein",
                          label: t("tts.summary.audiences.general"),
                        },
                        {
                          value: "fachpublikum",
                          label: t("tts.summary.audiences.expert"),
                        },
                        {
                          value: "management",
                          label: t("tts.summary.audiences.management"),
                        },
                      ]}
                      onChange={(value) => value && setSumAudience(value)}
                    />
                  </div>
                </label>
              </div>
            )}
            <div className="flex gap-2 items-center flex-wrap">
              {/* Transport per design system: round glyph buttons, exactly one
                primary. Reading aloud is playback, so it gets the same family
                as every audio player in the app — not text buttons. */}
              {/* Vollstaendige Transportzeile nach Katalog: von der Mitte nach
                aussen — Hauptschalter, daneben die Satzspruenge; hinter dem
                Trenner die Aktionen, die die Wiedergabe nicht fortbewegen.
                Statt ±15 s stehen hier Saetze: vorgelesener Text ist satzweise
                aufgebaut, eine Sekundenmarke gibt es darin nicht. */}
              <div className="mediabar mediabar--start">
                <button
                  type="button"
                  className="mbtn"
                  onClick={() => seekSentence(-1)}
                  disabled={!canResume && !speaking}
                  aria-label={t("tts.previousSentence")}
                >
                  <Glyph name="prev" />
                </button>
                <button
                  type="button"
                  className="mbtn mbtn--primary mbtn--lg"
                  onClick={speaking ? pauseSpeaking : speak}
                  disabled={!speaking && spokenText.trim().length === 0}
                  aria-label={speaking ? t("tts.pause") : t("tts.speak")}
                >
                  <Glyph name={speaking ? "pause" : "play"} />
                </button>
                <button
                  type="button"
                  className="mbtn"
                  onClick={() => seekSentence(1)}
                  disabled={!canResume && !speaking}
                  aria-label={t("tts.nextSentence")}
                >
                  <Glyph name="next" />
                </button>
                <span className="mediabar__sep" />
                <button
                  type="button"
                  className="mbtn"
                  onClick={stopSpeaking}
                  aria-label={t("tts.stop")}
                >
                  <Glyph name="stop" />
                </button>
                <span className="mediabar__sep" />
                {/* Tempo gehoert an die Transportleiste, nicht in die
                  Einstellungen: man merkt beim Hoeren, dass es zu langsam
                  ist, nicht vorher. Dieselbe Einstellung wie unten, nur hier
                  erreichbar. Bereich bewusst eng — Tempo entsteht per
                  Resampling und zieht die Tonhoehe mit. */}
                <div
                  className="w-28"
                  title={t("tts.settings.speedDescription")}
                >
                  <Select
                    value={String(getSetting("tts_speed") ?? 1.0)}
                    options={SPEEDS.map((value) => ({
                      value: String(value),
                      label: `${value.toFixed(2).replace(".", ",")}×`,
                    }))}
                    onChange={(value) =>
                      value && updateSetting("tts_speed", Number(value))
                    }
                    isClearable={false}
                  />
                </div>
                {/* Die Stimme dort, wo man sie wechselt: beim Hoeren. Wechsel
                  wirkt sofort — eine laufende Wiedergabe stellt am aktuellen
                  Satz um. Leerer Wert = Standardstimme (Seed). Verwaltung
                  (aufnehmen, importieren, loeschen) unten bei den
                  Einstellungen. */}
                <div className="w-40" title={t("tts.voices.title")}>
                  {/* Kennwert statt leerem Text fuer die Standardstimme:
                      "" gilt der Select-Komponente als "nichts gewaehlt" und
                      zeigte den Platzhalter "Select…" statt des Namens. */}
                  <Select
                    value={getSetting("tts_voice") ?? "@default"}
                    options={[
                      {
                        value: "@default",
                        label: t("tts.voices.defaultVoice"),
                      },
                      ...voices.map((id) => ({ value: id, label: id })),
                    ]}
                    onChange={(value) =>
                      updateSetting(
                        "tts_voice",
                        value === "@default" ? null : value,
                      )
                    }
                    isClearable={false}
                  />
                </div>
              </div>
              {/* Nur das Symbol: die Zeile ist eine Transportleiste, und ein
                Wort neben lauter Glyphen zieht das Auge auf die unwichtigste
                Schaltflaeche. Beschriftung wandert in title + aria-label. */}
              <Button
                variant="secondary"
                onClick={saveSpokenAudio}
                disabled={saving || spokenText.trim().length === 0}
                title={saving ? t("tts.savingAudio") : t("tts.saveAudio")}
                aria-label={saving ? t("tts.savingAudio") : t("tts.saveAudio")}
              >
                <Download width={16} height={16} />
              </Button>
              {saving && (
                <div className="flex items-center gap-2">
                  <div className="w-32 h-1.5 rounded-full bg-mid-gray/20 overflow-hidden">
                    <div
                      className="h-full bg-logo-primary transition-[width] duration-200"
                      style={{
                        width: exportProgress?.total
                          ? `${(exportProgress.position / exportProgress.total) * 100}%`
                          : "0%",
                      }}
                    />
                  </div>
                  <span className="text-xs text-text/60 tabular-nums">
                    {exportProgress?.total
                      ? t("tts.sentenceProgress", {
                          position: exportProgress.position,
                          total: exportProgress.total,
                        })
                      : t("tts.savingAudio")}
                  </span>
                  <button
                    type="button"
                    className="mbtn mbtn--sm"
                    onClick={cancelExport}
                    aria-label={t("tts.cancelExport")}
                  >
                    <Glyph name="stop" />
                  </button>
                </div>
              )}
              {speakProgress && (
                <span className="text-xs text-text/60">
                  {t("tts.sentenceProgress", {
                    position: speakProgress.position,
                    total: speakProgress.total,
                  })}
                </span>
              )}
            </div>
            {/* Sprecherwechsel und Tags sind Schreibregeln, keine
              Einstellungen — der aufklappbare Block steht deshalb bei dem
              Feld, in das man sie tippt. */}
            <details className="text-xs text-text/50">
              <summary className="cursor-pointer select-none transition-colors hover:text-text/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary rounded-sm">
                {t("tts.writingRules.title")}
              </summary>
              <div className="mt-1 space-y-1 ps-4">
                <p>{t("tts.dialogHint")}</p>
                <p>{t("tts.writingRules.tagsIntro")}</p>
                <ul className="list-disc space-y-0.5 ps-4">
                  <li>
                    <code className="rounded bg-logo-primary/15 px-1 text-text/70">
                      {t("tts.writingRules.example1")}
                    </code>
                  </li>
                  <li>
                    <code className="rounded bg-logo-primary/15 px-1 text-text/70">
                      {t("tts.writingRules.example2")}
                    </code>
                  </li>
                  <li>
                    <code className="rounded bg-logo-primary/15 px-1 text-text/70">
                      {t("tts.writingRules.example3")}
                    </code>
                  </li>
                </ul>
              </div>
            </details>
            {speaking && currentSentence && (
              <p className="text-sm italic text-text/70 border-s-2 border-logo-primary ps-2">
                {currentSentence}
              </p>
            )}
          </div>
        </SettingsGroup>

        <ReadingCard />

        <SettingsGroup title={t("tts.settingsTitle")}>
          <ShortcutInput shortcutId="speak_clipboard" grouped={true} />
          <Slider
            value={getSetting("tts_volume") ?? 1.0}
            onChange={(value) => updateSetting("tts_volume", value)}
            min={0}
            max={1}
            step={0.05}
            formatValue={(value) => `${Math.round(value * 100)}%`}
            label={t("tts.settings.volume")}
            description={t("tts.settings.volumeDescription")}
            grouped={true}
          />
          <ToggleSwitch
            checked={getSetting("tts_normalize") ?? true}
            onChange={(checked) => updateSetting("tts_normalize", checked)}
            isUpdating={isUpdating("tts_normalize")}
            label={t("tts.settings.normalize")}
            description={t("tts.settings.normalizeDescription")}
            grouped={true}
          />
          <ToggleSwitch
            checked={getSetting("tts_prewarm") ?? false}
            onChange={(checked) => updateSetting("tts_prewarm", checked)}
            isUpdating={isUpdating("tts_prewarm")}
            label={t("tts.settings.prewarm")}
            description={t("tts.settings.prewarmDescription")}
            grouped={true}
          />
          <ToggleSwitch
            checked={getSetting("tts_enhance") ?? true}
            onChange={(checked) => updateSetting("tts_enhance", checked)}
            isUpdating={isUpdating("tts_enhance")}
            label={t("tts.settings.enhance")}
            description={t("tts.settings.enhanceDescription")}
            grouped={true}
          />
          {(getSetting("tts_enhance") ?? true) && (
            <SettingContainer
              title={t("tts.settings.enhanceStrength")}
              description={t("tts.settings.enhanceStrengthDescription")}
              grouped={true}
              layout="horizontal"
            >
              <div className="w-40">
                <Select
                  value={getSetting("tts_enhance_strength") ?? "gentle"}
                  options={[
                    {
                      value: "gentle",
                      label: t("tts.settings.strengthGentle"),
                    },
                    {
                      value: "medium",
                      label: t("tts.settings.strengthMedium"),
                    },
                    {
                      value: "strong",
                      label: t("tts.settings.strengthStrong"),
                    },
                  ]}
                  onChange={(value) =>
                    value &&
                    updateSetting(
                      "tts_enhance_strength",
                      value as "gentle" | "medium" | "strong",
                    )
                  }
                  isClearable={false}
                />
              </div>
            </SettingContainer>
          )}
          <Slider
            value={getSetting("tts_speed") ?? 1.0}
            onChange={(value) => updateSetting("tts_speed", value)}
            min={0.5}
            max={2}
            step={0.05}
            formatValue={(value) => `${value.toFixed(2)}×`}
            label={t("tts.settings.speed")}
            description={t("tts.settings.speedDescription")}
            grouped={true}
          />
          <SettingContainer
            title={t("tts.settings.exportFormat")}
            description={t("tts.settings.exportFormatDescription")}
            grouped={true}
            layout="horizontal"
          >
            <div className="w-36">
              {/* Formatnamen sind Eigennamen — bewusst nicht übersetzt. */}
              <Select
                value={getSetting("tts_export_format") ?? "wav"}
                options={[
                  { value: "wav", label: "WAV" },
                  { value: "mp3", label: "MP3" },
                  { value: "opus", label: "Opus" },
                ]}
                isClearable={false}
                onChange={(value) => {
                  if (value) updateSetting("tts_export_format", value);
                }}
              />
            </div>
          </SettingContainer>
          <SettingContainer
            title={t("tts.settings.fishDir")}
            description={t("tts.settings.fishDirDescription")}
            grouped={true}
            layout="stacked"
          >
            <Input
              type="text"
              value={getSetting("tts_fish_dir") ?? ""}
              onChange={(e) => updateSetting("tts_fish_dir", e.target.value)}
              disabled={isUpdating("tts_fish_dir")}
              className="w-full"
            />
          </SettingContainer>
          <SettingContainer
            title={t("tts.settings.port")}
            description={t("tts.settings.portDescription")}
            grouped={true}
            layout="horizontal"
          >
            <Input
              type="number"
              min="1"
              max="65535"
              value={getSetting("tts_port") ?? 8080}
              onChange={(e) => {
                const value = parseInt(e.target.value, 10);
                if (!isNaN(value) && value > 0 && value <= 65535) {
                  updateSetting("tts_port", value);
                }
              }}
              disabled={isUpdating("tts_port")}
              className="w-24"
            />
          </SettingContainer>
          <SettingContainer
            title={t("tts.settings.seed")}
            description={t("tts.settings.seedDescription")}
            grouped={true}
            layout="horizontal"
          >
            {/* Der Seed bestimmt, wie die Standardstimme klingt. Er ist fest
              einstellbar, damit eine gefundene Stimme wiederholbar bleibt —
              und wuerfelbar, weil man sie nur durch Ausprobieren findet. Der
              gewuerfelte Wert landet sichtbar im Feld; genau der ist die
              Notiz, mit der man spaeter zurueckkommt. */}
            <div className="flex items-center gap-2">
              <Input
                type="number"
                value={getSetting("tts_seed") ?? 42}
                onChange={(e) => {
                  const value = parseInt(e.target.value, 10);
                  if (!isNaN(value)) updateSetting("tts_seed", value);
                }}
                disabled={isUpdating("tts_seed")}
                className="w-28"
              />
              <Button
                variant="secondary"
                size="sm"
                onClick={() =>
                  updateSetting(
                    "tts_seed",
                    Math.floor(Math.random() * 2_147_483_647) + 1,
                  )
                }
                disabled={isUpdating("tts_seed")}
              >
                <Dices width={14} height={14} />
                {t("tts.settings.rollSeed")}
              </Button>
              {/* Ein Seed ist fluechtig: wer weiterwuerfelt, verliert die
                Stimme, die ihm eben gefiel — und denselben Zahlenwert
                wiederzufinden ist aussichtslos. Speichern macht daraus eine
                benannte Stimme in der Auswahl. */}
              <Button
                variant="secondary"
                size="sm"
                onClick={() => setSaveSeedOpen(true)}
                disabled={savingSeed}
                title={t("tts.saveSeedHint")}
              >
                <Save width={14} height={14} />
                {t("tts.saveSeed")}
              </Button>
            </div>
          </SettingContainer>
          <SettingContainer
            title={t("tts.settings.idleMinutes")}
            description={t("tts.settings.idleMinutesDescription")}
            grouped={true}
            layout="horizontal"
          >
            <Input
              type="number"
              min="0"
              max="1440"
              value={getSetting("tts_idle_minutes") ?? 15}
              onChange={(e) => {
                const value = parseInt(e.target.value, 10);
                if (!isNaN(value) && value >= 0) {
                  updateSetting("tts_idle_minutes", value);
                }
              }}
              disabled={isUpdating("tts_idle_minutes")}
              className="w-24"
            />
          </SettingContainer>
          <ToggleSwitch
            checked={getSetting("tts_compile") ?? true}
            onChange={(checked) => updateSetting("tts_compile", checked)}
            isUpdating={isUpdating("tts_compile")}
            label={t("tts.settings.compile")}
            description={t("tts.settings.compileDescription")}
            grouped={true}
          />
          <ToggleSwitch
            checked={getSetting("tts_context_menu") ?? false}
            onChange={(checked) => updateSetting("tts_context_menu", checked)}
            isUpdating={isUpdating("tts_context_menu")}
            label={t("tts.settings.contextMenu")}
            description={t("tts.settings.contextMenuDescription")}
            grouped={true}
          />
          <SettingContainer
            title={t("tts.settings.maxChars")}
            description={t("tts.settings.maxCharsDescription")}
            grouped={true}
            layout="horizontal"
          >
            <Input
              type="number"
              min="100"
              max="100000"
              value={getSetting("tts_max_chars") ?? 5000}
              onChange={(e) => {
                const value = parseInt(e.target.value, 10);
                if (!isNaN(value) && value >= 100) {
                  updateSetting("tts_max_chars", value);
                }
              }}
              disabled={isUpdating("tts_max_chars")}
              className="w-24"
            />
          </SettingContainer>
        </SettingsGroup>

        {/* Verwaltung der Stimmen und der Stimmwechsler gehoeren zu den
          Einstellungen ans Ende: ausgewaehlt wird oben am Dropdown, hierher
          kommt man zum Aufnehmen, Importieren und Loeschen. */}
        <VoicesCard />

        <VoiceChangerCard />

        <Dialog
          open={saveSeedOpen}
          onOpenChange={(open) => {
            setSaveSeedOpen(open);
            if (!open) setSeedName("");
          }}
          title={t("tts.saveSeedTitle")}
          closeLabel={t("tts.stopConfirmCancel")}
          footer={
            <>
              <Button
                variant="secondary"
                onClick={() => setSaveSeedOpen(false)}
              >
                {t("tts.stopConfirmCancel")}
              </Button>
              <Button
                onClick={saveSeedVoice}
                disabled={savingSeed || !seedName.trim()}
              >
                {savingSeed ? t("tts.saveSeedBusy") : t("tts.saveSeed")}
              </Button>
            </>
          }
        >
          <div className="space-y-2">
            <p className="text-sm text-text/80">{t("tts.saveSeedBody")}</p>
            <Input
              type="text"
              value={seedName}
              onChange={(e) => setSeedName(e.target.value)}
              placeholder={t("tts.saveSeedPlaceholder")}
              className="w-full"
            />
          </div>
        </Dialog>

        <Dialog
          open={llmDialog}
          onOpenChange={setLlmDialog}
          title={t("tts.llm.dialogTitle")}
          closeLabel={t("tts.stopConfirmCancel")}
          footer={
            <>
              <Button variant="secondary" onClick={() => setLlmDialog(false)}>
                {t("tts.stopConfirmCancel")}
              </Button>
              <Button
                variant="secondary"
                onClick={llmWarmNow}
                disabled={llmWorking}
              >
                {t("tts.llm.warm")}
              </Button>
              <Button
                variant="danger"
                onClick={llmUnloadNow}
                disabled={llmWorking || llmLoaded.length === 0}
              >
                {t("tts.llm.unload")}
              </Button>
            </>
          }
        >
          <p className="text-sm text-text/80">
            {llmLoaded.length > 0
              ? t("tts.llm.dialogLoaded", { models: llmLoaded.join(", ") })
              : t("tts.llm.dialogEmpty")}
          </p>
        </Dialog>

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

        <Dialog
          open={confirmStop}
          onOpenChange={setConfirmStop}
          title={
            phase === "stopped" || phase === "error"
              ? t("tts.serverStartTitle")
              : t("tts.stopConfirmTitle")
          }
          closeLabel={t("tts.stopConfirmCancel")}
          footer={
            phase === "stopped" || phase === "error" ? (
              <>
                <Button
                  variant="secondary"
                  onClick={() => setConfirmStop(false)}
                >
                  {t("tts.stopConfirmCancel")}
                </Button>
                <Button
                  onClick={() => {
                    setConfirmStop(false);
                    void startServer();
                  }}
                >
                  {t("tts.serverStart")}
                </Button>
              </>
            ) : (
              <>
                <Button
                  variant="secondary"
                  onClick={() => setConfirmStop(false)}
                >
                  {t("tts.stopConfirmCancel")}
                </Button>
                <Button
                  variant="secondary"
                  onClick={() => {
                    setConfirmStop(false);
                    void restartServer();
                  }}
                >
                  {t("tts.stopConfirmRestart")}
                </Button>
                <Button
                  variant="danger"
                  onClick={() => {
                    setConfirmStop(false);
                    void killServer();
                  }}
                >
                  {t("tts.stopConfirmAccept")}
                </Button>
              </>
            )
          }
        >
          <p className="text-sm text-text/80">
            {phase === "stopped" || phase === "error"
              ? t("tts.serverStartBody")
              : phase === "starting"
                ? t("tts.stopConfirmBodyStarting")
                : t("tts.stopConfirmBody")}
          </p>
        </Dialog>
      </div>
      <FilesSidebar
        pageId={activePage}
        collapsed={filesCollapsed === "1"}
        onToggle={() => setFilesCollapsed(filesCollapsed === "1" ? "0" : "1")}
      />
    </div>
  );
};
