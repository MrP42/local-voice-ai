import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { createPortal } from "react-dom";
import { Check, Download, Mail, MessageSquare, Pencil, X } from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import {
  commands,
  events,
  type Citation,
  type Meeting,
  type MeetingSpeaker,
  type Participant,
  type StoredSegment,
} from "@/bindings";
import { Button } from "../../ui/Button";
import { Textarea } from "../../ui/Textarea";
import { TabList } from "../../ui/TabList";
import {
  AudioPlayer,
  AudioPlayerGroup,
  type AudioPlayerHandle,
} from "../../ui/AudioPlayer";
import Badge from "../../ui/Badge";
import { MinutesView } from "./MinutesView";
import { MyNotesView } from "./notes/MyNotesView";
import { EnhancedNotesView } from "./notes/EnhancedNotesView";
import { MeetingTemplatePicker } from "./notes/TemplatePicker";
import { RetranscribeControl } from "./RetranscribeControl";
import { SpeakerPopover } from "./SpeakerPopover";
import { Input } from "../../ui/Input";
import { translateMeetingError } from "./meetingErrors";
import { SOURCE_HIGHLIGHT_MS } from "@/lib/meetingNotes";
import { FollowupDialog } from "./FollowupDialog";
import { MeetingExportDialog } from "./MeetingExportDialog";
import { PeopleDialog } from "./people/PeopleDialog";
import { PersonPopover, type PersonRef } from "./people/PersonPopover";
import { orderParticipants } from "@/lib/meetingPeople";
import { useMeetingProgress } from "@/hooks/useMeetingJobs";
import { usePersistentState } from "@/hooks/usePersistentState";
import { JobPanel } from "./JobProgress";
import { audioTranscriptPlayer } from "./transcriptPlayer";

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

// Windows paths use backslashes; the old class `[\/]` matched only the
// forward slash, so a C:\... path came back whole.
const fileBaseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

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
  /** M4-P4e: Sprung aus einem Chat ausserhalb (global); `nonce` je Klick neu. */
  jumpRequest?: { citation: Citation; nonce: number } | null;
  /** M5-P5d: Popover einer Person -> Liste auf ihre Besprechungen eingrenzen. */
  onPersonFilter?: (person: PersonRef) => void;
  /** M5-P5d: Popover einer Person -> Chat ueber alle Besprechungen mit ihr. */
  onPersonAsk?: (person: PersonRef) => void;
}

export const MeetingDetail: React.FC<MeetingDetailProps> = ({
  meeting,
  slots,
  onShowTranscript,
  chatOpen,
  onChatToggle,
  onMeetingChange,
  jumpRequest,
  onPersonFilter,
  onPersonAsk,
}) => {
  const { t, i18n } = useTranslation();
  const meetingId = meeting.id;
  const meetingTitle = meeting.title;
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState(meetingTitle);
  const [titleError, setTitleError] = useState<string | null>(null);
  // Der letzte Reiter bleibt ueber Neuladen, Seitenwechsel und Neustart.
  const [midTab, setMidTab] = usePersistentState<MidTab>(
    "meetings.midTab",
    "notes",
    isMidTab,
  );
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
  const player = audioTranscriptPlayer(micPlayerRef, systemPlayerRef, hasAudio);

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
    if (!transcribing || !autoScroll || !following || !slots.transcript) {
      return;
    }
    const el = transcriptRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [segments, transcribing, autoScroll, following, slots.transcript]);

  useEffect(() => {
    setFollowing(true);
  }, [meetingId]);

  const onTranscriptScroll = () => {
    const el = transcriptRef.current;
    if (!el) return;
    // Am Ende (mit etwas Spiel) folgt das Mitscrollen wieder, sonst pausiert es.
    setFollowing(el.scrollHeight - el.scrollTop - el.clientHeight < 24);
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

  const loadSegments = useCallback(async () => {
    setLoading(true);
    const [result] = await Promise.all([
      commands.meetingsGetSegments(meetingId),
      loadSpeakers(),
    ]);
    setLoading(false);
    setEpochKey((k) => k + 1);
    if (result.status === "ok") {
      // Segments come back in segment_index order, which interleaves
      // channels for a live-recorded meeting — always sort by start_ms.
      setSegments([...result.data].sort((a, b) => a.start_ms - b.start_ms));
    }
  }, [meetingId, loadSpeakers]);

  useEffect(() => {
    void loadSegments();
  }, [loadSegments]);

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
  useEffect(() => {
    const un = events.meetingEvent.listen((e) => {
      const payload = e.payload;
      if (payload.kind === "levels" || payload.meeting_id !== meetingId) return;
      if (payload.kind === "reset") {
        setSegments([]);
        setEpochKey((k) => k + 1);
      } else if (payload.kind === "segments") {
        setSegments((prev) =>
          [...prev, ...payload.appended].sort(
            (a, b) => a.start_ms - b.start_ms,
          ),
        );
      } else if (payload.kind === "state") {
        // Statusfeld und Dauer/Audio-Pfade kommen aus dem Datensatz; nach
        // ready/failed/cancelled einmal frisch laden, damit Badge und Player
        // stimmen. P8a: auch beim Wechsel nach `processing` (Fortsetzen).
        const finished =
          payload.status === "ready" ||
          payload.status === "failed" ||
          payload.status === "cancelled";
        if (finished) void loadSegments();
        if (finished || payload.status === "processing") {
          void commands.meetingsList(0, 200).then((r) => {
            if (r.status !== "ok") return;
            const fresh = r.data.find((m) => m.id === meetingId);
            if (fresh) onMeetingChange(fresh);
          });
        }
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, [meetingId, loadSegments, onMeetingChange]);

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
  keepAtEnd.current = transcribing && autoScroll && following;
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

  const saveTitle = async () => {
    const next = titleDraft.trim();
    if (next === "" || next === meetingTitle) {
      setEditingTitle(false);
      setTitleDraft(meetingTitle);
      return;
    }
    const result = await commands.meetingsRename(meetingId, next);
    if (result.status === "error") {
      setTitleError(translateMeetingError(result.error, t));
      return;
    }
    setTitleError(null);
    setEditingTitle(false);
    onMeetingChange({ ...meeting, title: next });
  };

  const cancelTitleEdit = () => {
    setEditingTitle(false);
    setTitleDraft(meetingTitle);
    setTitleError(null);
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

  /** Kopf der Besprechung und die Reiter der Arbeitsflaeche (Mitte). */
  const contentPart = (
    <>
      {/* Title and origin are two different facts: the title is what the
      user calls this meeting, `source_path` is the file it was imported
      from. Renaming must not lose the second one, hence both lines. */}
      {editingTitle ? (
        <div className="flex flex-wrap items-center gap-2">
          <Input
            value={titleDraft}
            onChange={(e) => setTitleDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void saveTitle();
              if (e.key === "Escape") cancelTitleEdit();
            }}
            className="flex-1 min-w-[12rem]"
            autoFocus
            aria-label={t("meetings.detail.titleLabel")}
          />
          <Button size="sm" onClick={saveTitle}>
            <Check width={14} height={14} />
            {t("meetings.detail.save")}
          </Button>
          <Button size="sm" variant="secondary" onClick={cancelTitleEdit}>
            <X width={14} height={14} />
            {t("meetings.detail.cancel")}
          </Button>
        </div>
      ) : (
        <div className="flex items-start gap-2 group">
          <h3 className="text-base font-semibold break-words min-w-0 flex-1">
            {meetingTitle}
          </h3>
          <button
            type="button"
            onClick={() => {
              setTitleDraft(meetingTitle);
              setEditingTitle(true);
            }}
            title={t("meetings.detail.renameTitle")}
            className="p-1 rounded-md text-text/50 hover:text-logo-primary cursor-pointer shrink-0"
          >
            <Pencil width={14} height={14} />
          </button>
        </div>
      )}
      {meeting.source_path && (
        <p
          className="text-xs text-text/50 break-all -mt-2"
          title={meeting.source_path}
        >
          {fileBaseName(meeting.source_path)}
        </p>
      )}
      {titleError && <p className="text-sm text-red-400">{titleError}</p>}

      {participants.length > 0 && (
        <div
          className="flex flex-wrap items-center gap-1.5"
          role="group"
          aria-label={t("meetings.people.chipsLabel")}
          data-testid="participant-chips"
        >
          {orderParticipants(participants).map((participant) => (
            <PersonPopover
              key={participant.human_id}
              participant={participant}
              onFilter={(person) => onPersonFilter?.(person)}
              onAsk={(person) => onPersonAsk?.(person)}
              onManage={() => setPeopleOpen(true)}
            />
          ))}
        </div>
      )}

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

      <div className="grid grid-cols-[minmax(6rem,auto)_1fr] gap-x-4 gap-y-1 text-sm border border-mid-gray/20 rounded-md px-3 py-2">
        <span className="text-text/60">{t("meetings.meta.status")}</span>
        <span>
          <Badge variant={meeting.status === "ready" ? "success" : "secondary"}>
            {t(`meetings.status.${meeting.status}`, {
              defaultValue: meeting.status,
            })}
          </Badge>
        </span>
        <span className="text-text/60">{t("meetings.meta.source")}</span>
        <span>
          {t(`meetings.meta.sourceKind.${meeting.source}`, {
            defaultValue: meeting.source,
          })}
        </span>
        <span className="text-text/60">{t("meetings.meta.started")}</span>
        <span>
          {new Intl.DateTimeFormat(i18n.language, {
            dateStyle: "medium",
            timeStyle: "short",
          }).format(
            new Date((meeting.started_at ?? meeting.created_at) * 1000),
          )}
        </span>
        {meeting.duration_ms !== null && (
          <>
            <span className="text-text/60">{t("meetings.meta.duration")}</span>
            <span>{formatMmSs(meeting.duration_ms)}</span>
          </>
        )}
        {meeting.consent_confirmed_at !== null && (
          <>
            <span className="text-text/60">{t("meetings.meta.consent")}</span>
            <span>
              {new Intl.DateTimeFormat(i18n.language, {
                dateStyle: "medium",
                timeStyle: "short",
              }).format(new Date(meeting.consent_confirmed_at * 1000))}
            </span>
          </>
        )}
        {meeting.audio_retention_until !== null && (
          <>
            <span className="text-text/60">
              {t("meetings.meta.retentionUntil")}
            </span>
            <span>
              {new Intl.DateTimeFormat(i18n.language, {
                dateStyle: "medium",
                timeStyle: "short",
              }).format(new Date(meeting.audio_retention_until * 1000))}
            </span>
          </>
        )}
        <span className="text-text/60">{t("meetings.meta.segments")}</span>
        <span>{segments.length}</span>
      </div>

      <RetranscribeControl
        meeting={meeting}
        onFinished={loadSegments}
        busy={meeting.status === "processing" || jobProgress !== undefined}
      />

      {/* Die Reiter bleiben beim Scrollen der Arbeitsflaeche oben stehen. */}
      <div className="sticky top-0 z-10 -mx-4 bg-background px-4">
        <TabList
          tabs={midTabs}
          value={midTab}
          onChange={setMidTab}
          ariaLabel={t("meetings.layout.contentTabs")}
          className="border-b border-mid-gray/20"
        />
      </div>

      <div role="tabpanel" data-testid={`mid-panel-${midTab}`}>
        {midTab === "notes" || midTab === "ai" ? (
          <div className="space-y-3" ref={notesRef}>
            <div className="flex justify-end">
              <MeetingTemplatePicker meetingId={meetingId} />
            </div>
            {midTab === "notes" ? (
              <MyNotesView meeting={meeting} />
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

  /** Aktionen, Fortschritt und Wiedergabe (rechts oben, unter der Aufnahmekarte). */
  const controlsPart = (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="secondary"
          onClick={() => setExportOpen(true)}
          title={t("meetings.export.buttonTitle")}
          data-testid="export-open"
        >
          <Download width={14} height={14} aria-hidden="true" />
          {t("meetings.export.button")}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => setFollowupOpen(true)}
          title={t("meetings.followup.buttonTitle")}
          disabled={segments.length === 0}
          data-testid="followup-open"
        >
          <Mail width={14} height={14} aria-hidden="true" />
          {t("meetings.followup.button")}
        </Button>
        <Button
          size="sm"
          variant={chatOpen ? "primary-soft" : "secondary"}
          onClick={onChatToggle}
          title={t("meetings.chat.askTitle")}
          aria-pressed={chatOpen}
          aria-keyshortcuts="Control+J"
        >
          <MessageSquare width={14} height={14} aria-hidden="true" />
          {t("meetings.chat.ask")}
        </Button>
      </div>

      {jobProgress && <JobPanel progress={jobProgress} />}

      {(meeting.mic_audio_path || meeting.system_audio_path) && (
        <AudioPlayerGroup>
          {meeting.mic_audio_path && (
            <div className="space-y-1">
              <p className="text-xs text-text/60">
                {meeting.source === "import"
                  ? t("meetings.meta.audioImport")
                  : t("meetings.live.me")}
                {" · "}
                {fileBaseName(meeting.mic_audio_path)}
              </p>
              <AudioPlayer
                controlRef={micPlayerRef}
                src={convertFileSrc(meeting.mic_audio_path, "asset")}
                className="w-full"
              />
            </div>
          )}
          {meeting.system_audio_path && (
            <div className="space-y-1">
              <p className="text-xs text-text/60">
                {t("meetings.live.remote")}
                {" · "}
                {fileBaseName(meeting.system_audio_path)}
              </p>
              <AudioPlayer
                controlRef={systemPlayerRef}
                src={convertFileSrc(meeting.system_audio_path, "asset")}
                className="w-full"
              />
            </div>
          )}
        </AudioPlayerGroup>
      )}
    </>
  );

  /** Transkript (Reiter rechts unten): Werkzeugzeilen fest, die Liste scrollt. */
  const transcriptPart = (
    <>
      {transcribing && (
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
      {segments.length > 0 && (
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void copyTranscript(true)}
          >
            {copied === "meta"
              ? t("meetings.detail.copied")
              : t("meetings.detail.copyTranscript")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void copyTranscript(false)}
            title={t("meetings.detail.copyPlainHint")}
          >
            {copied === "plain"
              ? t("meetings.detail.copied")
              : t("meetings.detail.copyPlain")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={exportTranscript}
            title={t("meetings.detail.exportTranscript")}
            aria-label={t("meetings.detail.exportTranscript")}
          >
            <Download width={14} height={14} />
          </Button>
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
                  onClick={() => player.seek(segment.start_ms, segment.channel)}
                  title={t("meetings.detail.playFrom")}
                  className="text-xs text-text/40 w-10 shrink-0 pt-0.5 text-left tabular-nums hover:text-logo-primary hover:underline cursor-pointer"
                >
                  {formatMmSs(segment.start_ms)}
                </button>
              ) : (
                <span className="text-xs text-text/40 w-10 shrink-0 pt-0.5">
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
                    <Button size="sm" variant="secondary" onClick={cancelEdit}>
                      {t("meetings.detail.cancel")}
                    </Button>
                  </div>
                </div>
              ) : (
                <>
                  <p className="text-text/90 break-words flex-1">
                    {segment.text}
                  </p>
                  <button
                    type="button"
                    onClick={() => startEdit(segment)}
                    title={t("meetings.detail.editSegment")}
                    className="opacity-0 group-hover:opacity-100 p-1 rounded-md text-text/50 hover:text-logo-primary cursor-pointer shrink-0"
                  >
                    <Pencil width={14} height={14} />
                  </button>
                </>
              )}
            </div>
          ))}
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
    </>
  );
};
