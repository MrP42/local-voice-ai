import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { createPortal } from "react-dom";
import { ArrowDown, Pencil } from "lucide-react";
import { toast } from "sonner";
import { convertFileSrc } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import {
  commands,
  events,
  type Citation,
  type Folder,
  type Meeting,
  type MeetingSpeaker,
  type Participant,
  type StoredSegment,
} from "@/bindings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Textarea } from "../../ui/Textarea";
import {
  AudioPlayer,
  AudioPlayerGroup,
  type AudioPlayerHandle,
} from "../../ui/AudioPlayer";
import { MinutesView } from "./MinutesView";
import { MyNotesView } from "./notes/MyNotesView";
import { EnhancedNotesView } from "./notes/EnhancedNotesView";
import { MeetingTemplatePicker } from "./notes/TemplatePicker";
import { FolderPickerDialog } from "./search/FolderPickerDialog";
import { MeetingActions } from "./MeetingActions";
import { MeetingDetailsDialog } from "./MeetingDetailsDialog";
import { MeetingHeader } from "./MeetingHeader";
import { RetranscribeDialog } from "./RetranscribeDialog";
import { SpeakerPopover } from "./SpeakerPopover";
import { translateMeetingError } from "./meetingErrors";
import { enhanceErrorText, SOURCE_HIGHLIGHT_MS } from "@/lib/meetingNotes";
import { minutesErrorCode, minutesErrorDetail } from "@/lib/meetingMinutes";
import { notifyMeetingsChanged } from "@/lib/meetingsBus";
import { mergeSegments } from "@/lib/meetingSegments";
import { FollowupDialog } from "./FollowupDialog";
import { MeetingExportDialog } from "./MeetingExportDialog";
import { PeopleDialog } from "./people/PeopleDialog";
import type { PersonRef } from "./people/PersonPopover";
import { useMeetingProgress } from "@/hooks/useMeetingJobs";
import { usePersistentState } from "@/hooks/usePersistentState";
import { JobPanel } from "./JobProgress";
import { audioTranscriptPlayer } from "./transcriptPlayer";
import { useYoutubeSource } from "./youtube/useYoutubeSource";

const formatMmSs = (ms: number) => {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
};

const channelLabelKey = (channel: number) => {
  switch (channel) {
    case 0:
      return "meetings.live.me";
    case 1:
      return "meetings.live.remote";
    default:
      return "meetings.live.mixed";
  }
};

/** Reiter der Arbeitsflaeche (Mitte). Das Transkript steht rechts unten. */
type MidTab = "notes" | "ai" | "minutes";
const isMidTab = (value: string): value is MidTab =>
  value === "notes" || value === "ai" || value === "minutes";

/** Die Stellen der Aufnahmen-Seite, in die die Detailansicht ihre Teile legt. */
export interface MeetingDetailSlots {
  content: HTMLElement | null;
  controls: HTMLElement | null;
  transcript: HTMLElement | null;
}

interface MeetingDetailProps {
  meeting: Meeting;
  /** Bereiche der Aufnahmen-Seite, in die gerendert wird (Portale). Fehlt ein
      Bereich (eingeklappt, anderer Reiter), bleibt der Zustand trotzdem hier. */
  slots: MeetingDetailSlots;
  /** Transkript zeigen (Quellsprung aus KI-Notizen oder Chat). */
  onShowTranscript: () => void;
  /** Der Fragen-Reiter zeigt den Chat dieser Besprechung. */
  chatOpen: boolean;
  onChatToggle: () => void;
  /** Propagates a title change back to the list, which owns the record. */
  onMeetingChange: (meeting: Meeting) => void;
  /** Die Besprechung wurde ueber das Menue geloescht (die Seite waehlt ab). */
  onDeleted?: (id: string) => void;
  /** M4-P4e: Sprung aus einem Chat ausserhalb (global); `nonce` je Klick neu. */
  jumpRequest?: { citation: Citation; nonce: number } | null;
  /** M5-P5d: Popover einer Person -> Liste auf ihre Besprechungen eingrenzen. */
  onPersonFilter?: (person: PersonRef) => void;
  /** M5-P5d: Popover einer Person -> Chat ueber alle Besprechungen mit ihr. */
  onPersonAsk?: (person: PersonRef) => void;
  /** Diese Besprechung wird gerade aufgenommen (Notizblock mit Zeitstempel,
      Transkript waechst mit, keine Bearbeitung und kein Loeschen). */
  live?: boolean;
  /** Schmales Fenster: die Aktionen wandern ins Menue des Kopfes. */
  compact?: boolean;
}

export const MeetingDetail: React.FC<MeetingDetailProps> = ({
  meeting,
  slots,
  onShowTranscript,
  chatOpen,
  onChatToggle,
  onMeetingChange,
  onDeleted,
  jumpRequest,
  onPersonFilter,
  onPersonAsk,
  live = false,
  compact = false,
}) => {
  const { t } = useTranslation();
  const meetingId = meeting.id;
  const meetingTitle = meeting.title;
  // Dialoge und Anfragen aus Menue und Kopf.
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [retranscribeOpen, setRetranscribeOpen] = useState(false);
  const [templateOpen, setTemplateOpen] = useState(false);
  const [moveOpen, setMoveOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [renameNonce, setRenameNonce] = useState(0);
  // Projekte (= Ordner der obersten Ebene): alle, und die dieser Besprechung.
  const [allFolders, setAllFolders] = useState<Folder[]>([]);
  const [meetingFolderIds, setMeetingFolderIds] = useState<string[]>([]);
  // Der letzte Reiter bleibt ueber Neuladen, Seitenwechsel und Neustart.
  const [midTab, setMidTab] = usePersistentState<MidTab>(
    "meetings.midTab",
    "notes",
    isMidTab,
  );
  // Eine laufende Aufnahme zeigt den Notizblock; danach darf der Reiter wechseln.
  useEffect(() => {
    if (live) setMidTab("notes");
  }, [live, setMidTab]);
  const midTabs = [
    { id: "notes" as const, label: t("meetings.notes.tab") },
    { id: "ai" as const, label: t("meetings.notes.view.ai") },
    { id: "minutes" as const, label: t("meetings.detail.minutesTab") },
  ];
  const [segments, setSegments] = useState<StoredSegment[]>([]);
  // M3-P3c: Sprecher (Namen, Anteile), Epoche der Segmentnummern und Hinweise
  // zur Sprechertrennung (`metadata_json.diarize`).
  const [speakers, setSpeakers] = useState<MeetingSpeaker[]>([]);
  const [segmentEpoch, setSegmentEpoch] = useState<number | null>(null);
  const [speakerNotices, setSpeakerNotices] = useState<string[]>([]);
  // Quellsprung aus den KI-Notizen: das Segment bleibt kurz markiert.
  const [highlightIndex, setHighlightIndex] = useState<number | null>(null);
  const highlightTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const transcriptRef = useRef<HTMLDivElement>(null);
  // P8a: laufende Verarbeitung dieser Besprechung (Fortschritt, Pause, Stopp).
  const progressMap = useMeetingProgress();
  const jobProgress = progressMap[meetingId];
  // Nur das Transkript waechst mit: Notizen und Protokoll aendern es nicht.
  const transcribing =
    meeting.status === "processing" ||
    (jobProgress !== undefined &&
      jobProgress.phase !== "notes" &&
      jobProgress.phase !== "minutes");
  // Das Transkript waechst gerade: Aufnahme oder Verarbeitung.
  const growing = live || transcribing;
  // Automatisch mitscrollen: Schalter (gemerkt) und "folgt gerade". Blaettert
  // der Nutzer hoch, pausiert das Mitscrollen, bis er wieder ans Ende scrollt
  // oder den Schalter benutzt.
  const [autoScrollPref, setAutoScrollPref] = usePersistentState<"on" | "off">(
    "meetings.autoscroll",
    "on",
    (v) => v === "on" || v === "off",
  );
  const autoScroll = autoScrollPref === "on";
  const [following, setFollowing] = useState(true);
  const [continuing, setContinuing] = useState(false);
  const [continueError, setContinueError] = useState<string | null>(null);
  // Erhoeht sich, wenn sich die Segmente ersetzt haben koennten (Laden,
  // Neu-Transkription): die KI-Notizen lesen dann die Epoche neu.
  const [epochKey, setEpochKey] = useState(0);
  // Sprungmarken: Kanal 1 (Gegenseite) liegt in der Systemaufnahme, alles
  // andere (Mikrofon, Import als Mischkanal) im Mikrofon-/Import-Player.
  const micPlayerRef = useRef<AudioPlayerHandle>(null);
  const systemPlayerRef = useRef<AudioPlayerHandle>(null);
  const hasAudio = Boolean(
    meeting?.mic_audio_path || meeting?.system_audio_path,
  );
  // Ein Klick auf eine Zeitmarke spricht diesen Player an. Heute ist das das
  // Audio der Besprechung; ein YouTube-Player (#66) setzt hier seine eigene
  // Umsetzung ein, das Transkript bleibt unveraendert.
  // Eine YouTube-Besprechung hat statt des Audios den YouTube-Player (#66).
  const youtube = useYoutubeSource(meeting);
  const player =
    youtube.player ??
    audioTranscriptPlayer(micPlayerRef, systemPlayerRef, hasAudio);

  /**
   * Quelle einer KI-Notiz: das Transkript rechts zeigen, das Segment markieren
   * und (solange es einen Player gibt) ab der Stelle abspielen. Die KI-Notizen
   * bleiben dabei stehen.
   */
  const jumpToSource = (segmentIndex: number) => {
    const segment = segments.find((s) => s.segment_index === segmentIndex);
    if (!segment) return;
    onShowTranscript();
    setHighlightIndex(segmentIndex);
    if (highlightTimer.current) clearTimeout(highlightTimer.current);
    highlightTimer.current = setTimeout(
      () => setHighlightIndex(null),
      SOURCE_HIGHLIGHT_MS,
    );
    if (player.canSeek) player.seek(segment.start_ms, segment.channel);
  };

  useEffect(
    () => () => {
      if (highlightTimer.current) clearTimeout(highlightTimer.current);
    },
    [],
  );

  // P8a: neue Segmente waehrend der Verarbeitung -> ans Ende scrollen, solange
  // der Schalter an ist und der Nutzer nicht weggescrollt hat.
  useLayoutEffect(() => {
    if (!growing || !autoScroll || !following || !slots.transcript) {
      return;
    }
    const el = transcriptRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [segments, growing, autoScroll, following, slots.transcript]);

  useEffect(() => {
    setFollowing(true);
  }, [meetingId]);

  const onTranscriptScroll = () => {
    const el = transcriptRef.current;
    if (!el) return;
    // Am Ende (mit etwas Spiel) folgt das Mitscrollen wieder, sonst pausiert es.
    setFollowing(el.scrollHeight - el.scrollTop - el.clientHeight < 24);
  };

  /** "Zum Live-Ende": wieder mitlaufen und ans Ende springen. */
  const toLiveEnd = () => {
    setFollowing(true);
    const el = transcriptRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  };

  const continueProcessing = async () => {
    setContinuing(true);
    setContinueError(null);
    const result = await commands.meetingsContinue(meetingId);
    setContinuing(false);
    if (result.status === "error") {
      setContinueError(translateMeetingError(result.error, t));
    }
  };

  // Nach dem Wechsel in den Transkript-Reiter steht die Zeile erst im DOM.
  useEffect(() => {
    if (highlightIndex === null || !slots.transcript) return;
    transcriptRef.current
      ?.querySelector<HTMLElement>(`[data-segment-index="${highlightIndex}"]`)
      ?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [highlightIndex, slots.transcript]);

  // M4-P4e: Chat (Strg+J) und Belegsprung.
  // M6-P6c: Follow-up-Mail
  const [followupOpen, setFollowupOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const notesRef = useRef<HTMLDivElement>(null);
  const [noteTarget, setNoteTarget] = useState<{
    selector: string;
    nonce: number;
  } | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (
        (e.ctrlKey || e.metaKey) &&
        !e.shiftKey &&
        !e.altKey &&
        e.key.toLowerCase() === "j"
      ) {
        e.preventDefault();
        onChatToggle();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onChatToggle]);

  /**
   * Beleg aus dem Chat: Transkript-Stelle wie eine KI-Notizen-Quelle
   * (markieren + abspielen), Notizen-Beleg im Reiter Notizen bzw. KI-Notizen
   * markieren.
   */
  const jumpToCitation = (citation: Citation) => {
    if (citation.source === "user_notes" || citation.source === "ai_notes") {
      setMidTab(citation.source === "user_notes" ? "notes" : "ai");
      if (citation.ref_key) {
        const attr =
          citation.source === "user_notes" ? "data-block-id" : "data-entry-id";
        setNoteTarget({
          selector: `[${attr}="${CSS.escape(citation.ref_key)}"]`,
          nonce: Date.now(),
        });
      }
      return;
    }
    if (citation.segment_index !== null) jumpToSource(citation.segment_index);
    else onShowTranscript();
  };

  // Notizen laden nach dem Tabwechsel: kurz auf den Block warten, dann
  // markieren (wie das Segment, SOURCE_HIGHLIGHT_MS lang).
  useEffect(() => {
    if (!noteTarget) return;
    let tries = 0;
    let marked: HTMLElement | null = null;
    let clear: ReturnType<typeof setTimeout> | null = null;
    const unmark = () => {
      if (!marked) return;
      delete marked.dataset.highlighted;
      marked.classList.remove("bg-logo-primary/25", "rounded-md");
      marked = null;
    };
    const poll = setInterval(() => {
      const el = notesRef.current?.querySelector<HTMLElement>(
        noteTarget.selector,
      );
      if (!el && ++tries < 30) return;
      clearInterval(poll);
      if (!el) return;
      marked = el;
      el.dataset.highlighted = "true";
      el.classList.add("bg-logo-primary/25", "rounded-md");
      el.scrollIntoView({ block: "center", behavior: "smooth" });
      clear = setTimeout(unmark, SOURCE_HIGHLIGHT_MS);
    }, 100);
    return () => {
      clearInterval(poll);
      if (clear) clearTimeout(clear);
      unmark();
    };
  }, [noteTarget]);

  const [loading, setLoading] = useState(true);
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [copied, setCopied] = useState<"meta" | "plain" | null>(null);
  const [transcriptError, setTranscriptError] = useState<string | null>(null);
  const [editText, setEditText] = useState("");
  const [saving, setSaving] = useState(false);

  /** Sprecher, Epoche und Hinweise; ein Fehler lässt den alten Stand stehen. */
  const loadSpeakers = useCallback(async () => {
    const [list, epoch, notices] = await Promise.all([
      commands.meetingSpeakersList(meetingId),
      commands.meetingsSegmentEpoch(meetingId),
      commands.meetingSpeakerNotices(meetingId),
    ]);
    if (list.status === "ok") setSpeakers(list.data ?? []);
    if (epoch.status === "ok") setSegmentEpoch(epoch.data);
    if (notices.status === "ok") setSpeakerNotices(notices.data ?? []);
  }, [meetingId]);

  // Ladevorgang und Ereignisse laufen nebeneinander (bei einer laufenden
  // Aufnahme wachsen die Segmente waehrend des Ladens weiter). Jeder Ladevorgang
  // bekommt eine Nummer; `liveBuffer` sammelt, was per Ereignis seit seinem
  // Beginn eintraf. Das Ergebnis ist die Vereinigung beider nach
  // `segment_index`: nichts doppelt, nichts verloren. Ein `reset` (Neu-
  // Transkription) macht eine laufende Ladung ungueltig.
  const loadSeq = useRef(0);
  const liveBuffer = useRef<StoredSegment[] | null>(null);
  const loadSegments = useCallback(async () => {
    const seq = ++loadSeq.current;
    liveBuffer.current = [];
    setLoading(true);
    const [result] = await Promise.all([
      commands.meetingsGetSegments(meetingId),
      loadSpeakers(),
    ]);
    if (seq !== loadSeq.current) return;
    const arrived = liveBuffer.current ?? [];
    liveBuffer.current = null;
    setLoading(false);
    setEpochKey((k) => k + 1);
    if (result.status === "ok") {
      // Segments come back in segment_index order, which interleaves
      // channels for a live-recorded meeting - always sort by start_ms.
      setSegments(mergeSegments(mergeSegments([], result.data), arrived));
    }
  }, [meetingId, loadSpeakers]);
  const loadSegmentsRef = useRef(loadSegments);
  loadSegmentsRef.current = loadSegments;

  // M5-P5d: Teilnehmende (Kalender, benannte Sprecher) als Chips in der Kopfzeile.
  const [participants, setParticipants] = useState<Participant[]>([]);
  const [peopleOpen, setPeopleOpen] = useState(false);
  const loadParticipants = useCallback(async () => {
    const result = await commands.meetingParticipants(meetingId);
    if (result.status === "ok") setParticipants(result.data ?? []);
  }, [meetingId]);
  useEffect(() => {
    void loadParticipants();
  }, [loadParticipants]);

  // M4-P4e: Sprung aus einem globalen Chat erst, wenn die Segmente da sind.
  const handledJump = useRef<number | null>(null);
  useEffect(() => {
    if (
      !jumpRequest ||
      loading ||
      handledJump.current === jumpRequest.nonce ||
      jumpRequest.citation.meeting_id !== meetingId
    )
      return;
    handledJump.current = jumpRequest.nonce;
    jumpToCitation(jumpRequest.citation);
    // jumpToCitation liest den aktuellen Stand; ausloesen nur je Anfrage.
  }, [jumpRequest?.nonce, loading, meetingId]);

  // Live mitlesen: Import, Aufnahme und Neu-Transkription schreiben Block
  // fuer Block und melden jeden ueber `meetingEvent`. Ohne diesen Hoerer
  // zeigte die Detailseite nur den Stand beim Oeffnen — wer waehrend der
  // Transkription zusah, sah nichts wachsen (17.09.2026).
  // Der Hoerer steht, bevor die Segmente geladen werden: ein Satz, der
  // dazwischen entsteht, geht so weder beim Laden noch beim Zuhoeren verloren.
  const onMeetingChangeRef = useRef(onMeetingChange);
  onMeetingChangeRef.current = onMeetingChange;
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    const listening = events.meetingEvent.listen((e) => {
      const payload = e.payload;
      if (payload.kind === "levels" || payload.meeting_id !== meetingId) return;
      if (payload.kind === "reset") {
        loadSeq.current += 1;
        liveBuffer.current = null;
        setLoading(false);
        setSegments([]);
        setEpochKey((k) => k + 1);
      } else if (payload.kind === "segments") {
        liveBuffer.current?.push(...payload.appended);
        setSegments((prev) => mergeSegments(prev, payload.appended));
      } else if (payload.kind === "state") {
        // Statusfeld und Dauer/Audio-Pfade kommen aus dem Datensatz; nach
        // ready/failed/cancelled einmal frisch laden, damit Badge und Player
        // stimmen. P8a: auch beim Wechsel nach `processing` (Fortsetzen).
        const finished =
          payload.status === "ready" ||
          payload.status === "failed" ||
          payload.status === "cancelled";
        if (finished) void loadSegmentsRef.current();
        if (finished || payload.status === "processing") {
          void commands.meetingsList(0, 200).then((r) => {
            if (r.status !== "ok") return;
            const fresh = r.data.find((m) => m.id === meetingId);
            if (fresh) onMeetingChangeRef.current(fresh);
          });
        }
      }
    });
    void listening.then(
      (un) => {
        if (cancelled) un();
        else {
          unlisten = un;
          void loadSegmentsRef.current();
        }
      },
      // Ohne Hoerer wenigstens den gespeicherten Stand zeigen.
      () => {
        if (!cancelled) void loadSegmentsRef.current();
      },
    );
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [meetingId]);

  // M3-P3c: Namen, Zusammenführen und Umhängen (auch aus anderen Ansichten):
  // Segmente tragen neue Sprecher, die Sprecherliste neue Labels.
  useEffect(() => {
    const un = events.speakersChanged.listen((e) => {
      if (e.payload.meeting_id !== meetingId) return;
      void commands.meetingsGetSegments(meetingId).then((result) => {
        if (result.status === "ok") {
          setSegments([...result.data].sort((a, b) => a.start_ms - b.start_ms));
        }
      });
      void loadSpeakers();
      // Benannte Sprecher sind Teilnehmende (M5-P5d).
      void loadParticipants();
    });
    return () => {
      un.then((f) => f());
    };
  }, [meetingId, loadSpeakers, loadParticipants]);

  /** Nach einer Änderung im Popover: Segmente und Sprecher frisch holen. */
  const onSpeakersChanged = useCallback(() => {
    void commands.meetingsGetSegments(meetingId).then((result) => {
      if (result.status === "ok") {
        setSegments([...result.data].sort((a, b) => a.start_ms - b.start_ms));
      }
    });
    void loadSpeakers();
    void loadParticipants();
  }, [meetingId, loadSpeakers, loadParticipants]);

  // Aendert sich die Hoehe der Liste (Fortschrittsblock waechst, Fenster wird
  // kleiner), bleibt ein mitlaufendes Transkript am Ende. Die Liste fuellt die
  // Resthoehe ihrer Spalte und hat keine feste Hoehe mehr, die das verhinderte.
  const keepAtEnd = useRef(false);
  keepAtEnd.current = growing && autoScroll && following;
  const hasList = !loading && segments.length > 0;
  useEffect(() => {
    const el = transcriptRef.current;
    if (!el || !slots.transcript || !hasList) return;
    const observer = new ResizeObserver(() => {
      if (keepAtEnd.current) el.scrollTop = el.scrollHeight;
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [slots.transcript, hasList]);

  /** Neuer Titel aus dem Kopf: Fehlertext oder `null`. */
  const renameTo = async (next: string): Promise<string | null> => {
    const result = await commands.meetingsRename(meetingId, next);
    if (result.status === "error") {
      return translateMeetingError(result.error, t);
    }
    onMeetingChange({ ...meeting, title: next });
    return null;
  };

  // F2 benennt um, solange kein Eingabefeld den Fokus hat.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "F2" || e.defaultPrevented) return;
      // Hinter einem offenen Dialog nichts umbenennen.
      if (document.querySelector('[role="dialog"][aria-modal="true"]')) return;
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.isContentEditable ||
          ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName))
      ) {
        return;
      }
      e.preventDefault();
      setRenameNonce((n) => n + 1);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  const loadFolders = useCallback(async () => {
    const [all, mine] = await Promise.all([
      commands.meetingFoldersList(),
      commands.meetingsGetFolders(meetingId),
    ]);
    if (all.status === "ok") setAllFolders(all.data ?? []);
    if (mine.status === "ok") setMeetingFolderIds(mine.data ?? []);
  }, [meetingId]);
  useEffect(() => {
    void loadFolders();
  }, [loadFolders]);
  const projectNames = allFolders
    .filter((folder) => meetingFolderIds.includes(folder.id))
    .map((folder) => folder.name);

  const confirmDelete = async () => {
    setDeleteOpen(false);
    const result = await commands.meetingsDelete(meetingId);
    if (result.status === "error") {
      toast.error(t("meetings.errors.deleteFailed"));
      return;
    }
    notifyMeetingsChanged();
    onDeleted?.(meetingId);
  };

  /** KI-Notizen bzw. Protokoll neu erzeugen: Reiter zeigen, dann starten. */
  const regenerateNotes = () => {
    setMidTab("ai");
    void commands.meetingNotesEnhance(meetingId, null).then((result) => {
      if (result.status === "error" && result.error !== "stopped") {
        const text = enhanceErrorText(result.error);
        toast.error(t(text.key, text.params));
      }
    });
  };
  const regenerateMinutes = () => {
    setMidTab("minutes");
    void commands.meetingsGenerateMinutes(meetingId, null).then((result) => {
      if (result.status === "error") {
        const code = minutesErrorCode(result.error);
        if (code === "minutes_busy" || code === "minutes_cancelled") return;
        toast.error(
          t(`meetings.minutes.errors.${code}`, {
            error: minutesErrorDetail(result.error),
            defaultValue: result.error,
          }),
        );
      }
    });
  };

  /**
   * Steht auf jeder Zeile dieselbe Quelle, unterscheidet die Spalte nichts
   * und kostet nur Platz und Aufmerksamkeit. Erst wenn Mikrofon und
   * Gegenseite getrennt vorliegen, traegt sie eine Information.
   */
  const hasSpeakers = segments.some((s) => s.speaker_index !== null);
  const showChannels =
    hasSpeakers || new Set(segments.map((s) => s.channel)).size > 1;

  /** Wer spricht: der Sprechername (oder "Gegenseite 2"), sonst der Kanal. */
  const speakerOf = (segment: StoredSegment) =>
    segment.speaker_index === null
      ? undefined
      : speakers.find(
          (s) =>
            s.channel === segment.channel &&
            s.speaker_index === segment.speaker_index,
        );
  const whoLabel = (segment: StoredSegment) =>
    speakerOf(segment)?.label ?? t(channelLabelKey(segment.channel));

  /** `withMeta` false liefert den blanken Text — ohne Zeitstempel, ohne
   *  Quelle, mit Leerzeile zwischen den Abschnitten, damit er sich als
   *  Fliesstext weiterverwenden laesst. */
  const transcriptText = (withMeta: boolean) =>
    segments
      .map((s) => {
        if (!withMeta) return s.text;
        const who = showChannels ? `${whoLabel(s)} ` : ``;
        return `${who}[${formatMmSs(s.start_ms)}]: ${s.text}`;
      })
      .join(withMeta ? "\n" : "\n\n");

  const copyTranscript = async (withMeta: boolean) => {
    try {
      await navigator.clipboard.writeText(transcriptText(withMeta));
      setCopied(withMeta ? "meta" : "plain");
      setTimeout(() => setCopied(null), 2000);
    } catch (e) {
      setTranscriptError(String(e));
    }
  };

  const exportTranscript = async () => {
    setTranscriptError(null);
    const target = await save({
      defaultPath: `${meetingTitle || "transkript"}.docx`,
      filters: [
        { name: "Word", extensions: ["docx"] },
        { name: "Text", extensions: ["txt"] },
        { name: "Markdown", extensions: ["md"] },
      ],
    });
    if (typeof target !== "string") return;
    // Wie beim Protokoll: geschrieben wird im Backend, weil das fs-Plugin
    // nur $APPDATA zulaesst. Der Sprecher steht fett vor seinem Beitrag,
    // damit die Word-Fassung als Mitschrift lesbar ist und nicht als Liste.
    const body = segments
      .map((s) => `**${whoLabel(s)} [${formatMmSs(s.start_ms)}]:** ${s.text}`)
      .join("\n\n");
    const result = await commands.meetingsExportDocument(target, body);
    if (result.status !== "ok") {
      setTranscriptError(result.error);
    }
  };

  const startEdit = (segment: StoredSegment) => {
    setEditingIndex(segment.segment_index);
    setEditText(segment.text);
  };

  const cancelEdit = () => {
    setEditingIndex(null);
    setEditText("");
  };

  const saveEdit = async (segmentIndex: number) => {
    setSaving(true);
    const result = await commands.meetingsUpdateSegment(
      meetingId,
      segmentIndex,
      editText,
    );
    setSaving(false);
    if (result.status === "ok") {
      setSegments((prev) =>
        prev.map((s) =>
          s.segment_index === segmentIndex ? { ...s, text: editText } : s,
        ),
      );
      setEditingIndex(null);
      setEditText("");
    }
  };

  /** Symbolzeile mit Menue (breit) bzw. nur das Menue mit allen Aktionen (schmal). */
  const actions = (menuOnly: boolean) => (
    <MeetingActions
      menuOnly={menuOnly}
      hasSegments={segments.length > 0}
      hasAudio={hasAudio}
      busy={
        live || meeting.status === "processing" || jobProgress !== undefined
      }
      live={live}
      chatOpen={chatOpen}
      copied={copied === "meta"}
      onExport={() => setExportOpen(true)}
      onFollowup={() => setFollowupOpen(true)}
      onCopy={() => void copyTranscript(true)}
      onPeople={() => setPeopleOpen(true)}
      onChatToggle={onChatToggle}
      onRetranscribe={() => setRetranscribeOpen(true)}
      onRegenNotes={regenerateNotes}
      onRegenMinutes={regenerateMinutes}
      onTemplate={() => setTemplateOpen(true)}
      onRename={() => setRenameNonce((n) => n + 1)}
      onMove={() => setMoveOpen(true)}
      onCopyPlain={() => void copyTranscript(false)}
      onExportTranscript={() => void exportTranscript()}
      onDetails={() => setDetailsOpen(true)}
      onDelete={() => setDeleteOpen(true)}
    />
  );

  /** Kopf der Besprechung und die Reiter der Arbeitsflaeche (Mitte). */
  const contentPart = (
    <>
      {youtube.panel}
      <MeetingHeader
        meeting={meeting}
        progress={jobProgress}
        participants={participants}
        projectNames={projectNames}
        onRename={renameTo}
        renameNonce={renameNonce}
        onOpenDetails={() => setDetailsOpen(true)}
        onOpenProjects={() => setMoveOpen(true)}
        onManagePeople={() => setPeopleOpen(true)}
        onPersonFilter={(person) => onPersonFilter?.(person)}
        onPersonAsk={(person) => onPersonAsk?.(person)}
        tabs={midTabs}
        tab={midTab}
        onTab={(id) => {
          if (isMidTab(id)) setMidTab(id);
        }}
        tabsLabel={t("meetings.layout.contentTabs")}
        menu={compact ? actions(true) : undefined}
      />

      {meeting.status === "cancelled" && !jobProgress && (
        <div
          data-testid="cancelled-panel"
          className="space-y-2 rounded-md border border-mid-gray/20 px-3 py-2"
        >
          <p className="text-sm font-medium">
            {t("meetings.progress.cancelledTitle")}
          </p>
          <p className="text-xs text-text/70">
            {t("meetings.progress.cancelledBody", { count: segments.length })}
          </p>
          {hasAudio && (
            <Button
              size="sm"
              variant="secondary"
              data-testid="job-continue"
              onClick={() => void continueProcessing()}
              disabled={continuing}
            >
              {t("meetings.progress.continue")}
            </Button>
          )}
          {continueError && (
            <p
              className="text-sm text-red-400"
              data-testid="job-continue-error"
            >
              {continueError}
            </p>
          )}
        </div>
      )}

      <div role="tabpanel" data-testid={`mid-panel-${midTab}`}>
        {midTab === "notes" || midTab === "ai" ? (
          <div className="space-y-3" ref={notesRef}>
            {midTab === "notes" ? (
              <MyNotesView meeting={meeting} live={live} compact={compact} />
            ) : (
              <EnhancedNotesView
                meeting={meeting}
                segments={segments}
                epochKey={epochKey}
                hasAudio={hasAudio}
                onJumpToSource={jumpToSource}
              />
            )}
          </div>
        ) : (
          <MinutesView meetingId={meetingId} meetingTitle={meetingTitle} />
        )}
      </div>
    </>
  );

  /** Wiedergabe, Fortschritt und Symbolzeile (rechts oben, unter der Aufnahmezeile). */
  const controlsPart = (
    <>
      {hasAudio && !live && (
        <AudioPlayerGroup>
          {meeting.mic_audio_path && (
            <div className="space-y-0.5" data-testid="rec-player">
              {meeting.system_audio_path && (
                <p className="text-xs text-text/60">
                  {meeting.source === "import"
                    ? t("meetings.meta.audioImport")
                    : t("meetings.live.me")}
                </p>
              )}
              <AudioPlayer
                compact
                controlRef={micPlayerRef}
                src={convertFileSrc(meeting.mic_audio_path, "asset")}
                className="w-full"
              />
            </div>
          )}
          {meeting.system_audio_path && (
            <div className="space-y-0.5" data-testid="rec-player-system">
              {meeting.mic_audio_path && (
                <p className="text-xs text-text/60">
                  {t("meetings.live.remote")}
                </p>
              )}
              <AudioPlayer
                compact
                controlRef={systemPlayerRef}
                src={convertFileSrc(meeting.system_audio_path, "asset")}
                className="w-full"
              />
            </div>
          )}
        </AudioPlayerGroup>
      )}

      {jobProgress && <JobPanel progress={jobProgress} />}

      {!compact && actions(false)}
    </>
  );

  /** Transkript (Reiter rechts unten): Werkzeugzeilen fest, die Liste scrollt. */
  const transcriptPart = (
    <>
      {growing && (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <label className="flex cursor-pointer items-center gap-2 text-xs text-text/80">
            <input
              type="checkbox"
              data-testid="autoscroll-toggle"
              checked={autoScroll}
              onChange={(e) => {
                setAutoScrollPref(e.target.checked ? "on" : "off");
                // Der Schalter selbst holt das Mitscrollen zurueck.
                if (e.target.checked) setFollowing(true);
              }}
            />
            {t("meetings.detail.autoscroll")}
          </label>
          {autoScroll && !following && (
            <span
              className="text-xs text-text/50"
              data-testid="autoscroll-paused"
            >
              {t("meetings.detail.autoscrollPaused")}
            </span>
          )}
        </div>
      )}
      {transcriptError && (
        <p className="text-sm text-red-400">{transcriptError}</p>
      )}
      {speakerNotices.length > 0 && (
        <ul
          className="space-y-0.5 text-xs text-text/50"
          data-testid="speaker-notices"
        >
          {speakerNotices.map((code) => (
            <li key={code} data-notice={code}>
              {t(`meetings.speakers.notices.${code}`, {
                defaultValue: code,
              })}
            </li>
          ))}
        </ul>
      )}
      {loading ? (
        <p className="text-sm text-text/60 text-center py-3">
          {t("meetings.list.loading")}
        </p>
      ) : segments.length === 0 ? (
        <p className="text-sm text-text/60">{t("meetings.live.empty")}</p>
      ) : (
        <div className="relative flex min-h-0 flex-1 flex-col">
          <div
            ref={transcriptRef}
            data-testid="transcript-scroll"
            onScroll={onTranscriptScroll}
            className="min-h-0 flex-1 space-y-2 overflow-y-auto"
          >
            {segments.map((segment) => (
              <div
                key={segment.segment_index}
                data-segment-index={segment.segment_index}
                data-highlighted={
                  highlightIndex === segment.segment_index ? "true" : undefined
                }
                className={`flex gap-2 items-start text-sm group rounded-md px-1 -mx-1 transition-colors ${
                  highlightIndex === segment.segment_index
                    ? "bg-logo-primary/25 ring-1 ring-logo-primary/50"
                    : ""
                }`}
              >
                {player.canSeek ? (
                  <button
                    type="button"
                    data-act="seek"
                    data-seek-ms={segment.start_ms}
                    onClick={() =>
                      player.seek(segment.start_ms, segment.channel)
                    }
                    title={t("meetings.detail.playFrom")}
                    className="text-xs text-text/60 w-10 shrink-0 pt-0.5 text-left tabular-nums hover:text-logo-primary hover:underline cursor-pointer"
                  >
                    {formatMmSs(segment.start_ms)}
                  </button>
                ) : (
                  <span className="text-xs text-text/60 w-10 shrink-0 pt-0.5">
                    {formatMmSs(segment.start_ms)}
                  </span>
                )}
                {showChannels && (
                  <span
                    className={`text-xs text-text/50 shrink-0 pt-0.5 truncate ${
                      hasSpeakers ? "w-24" : "w-16"
                    }`}
                    title={whoLabel(segment)}
                  >
                    {speakerOf(segment) ? (
                      <SpeakerPopover
                        meetingId={meetingId}
                        segment={segment}
                        speaker={speakerOf(segment)!}
                        speakers={speakers}
                        epoch={segmentEpoch}
                        onChanged={onSpeakersChanged}
                        className="max-w-full"
                      />
                    ) : (
                      t(channelLabelKey(segment.channel))
                    )}
                  </span>
                )}
                {editingIndex === segment.segment_index ? (
                  <div className="flex-1 space-y-1">
                    <Textarea
                      value={editText}
                      onChange={(e) => setEditText(e.target.value)}
                      rows={2}
                      className="w-full"
                      autoFocus
                    />
                    <div className="flex gap-2">
                      <Button
                        size="sm"
                        onClick={() => saveEdit(segment.segment_index)}
                        disabled={saving}
                      >
                        {t("meetings.detail.save")}
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={cancelEdit}
                      >
                        {t("meetings.detail.cancel")}
                      </Button>
                    </div>
                  </div>
                ) : (
                  <>
                    <p className="text-text/90 break-words flex-1">
                      {segment.text}
                    </p>
                    {!live && (
                      <button
                        type="button"
                        onClick={() => startEdit(segment)}
                        title={t("meetings.detail.editSegment")}
                        className="opacity-0 group-hover:opacity-100 p-1 rounded-md text-text/50 hover:text-logo-primary cursor-pointer shrink-0"
                      >
                        <Pencil width={14} height={14} />
                      </button>
                    )}
                  </>
                )}
              </div>
            ))}
          </div>
          {growing && !following && (
            <Button
              size="sm"
              variant="secondary"
              className="absolute bottom-2 end-3 shadow-md"
              data-testid="follow-live"
              onClick={toLiveEnd}
            >
              <ArrowDown width={14} height={14} aria-hidden="true" />
              {t("meetings.detail.toLiveEnd")}
            </Button>
          )}
        </div>
      )}
    </>
  );

  // Die Teile wandern per Portal in ihre Bereiche der Aufnahmen-Seite; der
  // Zustand (Segmente, Player, Auswahl) bleibt hier an einer Stelle.
  return (
    <>
      {slots.content && createPortal(contentPart, slots.content)}
      {slots.controls && createPortal(controlsPart, slots.controls)}
      {slots.transcript && createPortal(transcriptPart, slots.transcript)}
      <MeetingExportDialog
        open={exportOpen}
        onOpenChange={setExportOpen}
        meetingId={meetingId}
        meetingTitle={meetingTitle}
      />
      <FollowupDialog
        open={followupOpen}
        onOpenChange={setFollowupOpen}
        meetingId={meetingId}
      />
      <PeopleDialog
        open={peopleOpen}
        onOpenChange={setPeopleOpen}
        onChanged={() => void loadParticipants()}
      />
      <MeetingDetailsDialog
        open={detailsOpen}
        onOpenChange={setDetailsOpen}
        meeting={meeting}
        progress={jobProgress}
        segmentCount={segments.length}
        projectNames={projectNames}
      />
      <RetranscribeDialog
        open={retranscribeOpen}
        onOpenChange={setRetranscribeOpen}
        meeting={meeting}
        onFinished={loadSegments}
      />
      <Dialog
        open={templateOpen}
        onOpenChange={setTemplateOpen}
        title={t("meetings.actions.templateTitle")}
        description={t("meetings.actions.templateBody")}
        closeLabel={t("meetings.actions.close")}
        footer={
          <Button onClick={() => setTemplateOpen(false)}>
            {t("meetings.actions.close")}
          </Button>
        }
      >
        <div data-testid="template-dialog">
          <MeetingTemplatePicker meetingId={meetingId} menuPortal />
        </div>
      </Dialog>
      <FolderPickerDialog
        meeting={moveOpen ? meeting : null}
        folders={allFolders}
        onClose={() => setMoveOpen(false)}
        onSaved={() => {
          void loadFolders();
          notifyMeetingsChanged();
        }}
      />
      <Dialog
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        title={t("meetings.list.deleteConfirmTitle")}
        closeLabel={t("meetings.list.cancel")}
        footer={
          <>
            <Button variant="secondary" onClick={() => setDeleteOpen(false)}>
              {t("meetings.list.cancel")}
            </Button>
            <Button
              variant="danger"
              data-testid="meeting-delete-confirm"
              onClick={() => void confirmDelete()}
            >
              {t("meetings.list.deleteButton")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {t("meetings.list.deleteConfirm")}
        </p>
      </Dialog>
    </>
  );
};
