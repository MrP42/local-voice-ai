import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowLeft,
  Check,
  Download,
  Mail,
  MessageSquare,
  Pencil,
  X,
} from "lucide-react";
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
import { SettingsGroup } from "../../ui/SettingsGroup";
import { Button } from "../../ui/Button";
import { Textarea } from "../../ui/Textarea";
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
import { ChatPanel } from "./chat/ChatPanel";
import { FollowupDialog } from "./FollowupDialog";
import { MeetingExportDialog } from "./MeetingExportDialog";
import { PeopleDialog } from "./people/PeopleDialog";
import { PersonPopover, type PersonRef } from "./people/PersonPopover";
import { orderParticipants } from "@/lib/meetingPeople";

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

type Tab = "notes" | "transcript" | "minutes";
type NotesView = "mine" | "ai";

// Windows paths use backslashes; the old class `[\/]` matched only the
// forward slash, so a C:\... path came back whole.
const fileBaseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

interface MeetingDetailProps {
  meeting: Meeting;
  onBack: () => void;
  /** Propagates a title change back to the list, which owns the record. */
  onMeetingChange: (meeting: Meeting) => void;
  /** M4-P4e: Sprung aus einem Chat ausserhalb (global); `nonce` je Klick neu. */
  jumpRequest?: { citation: Citation; nonce: number } | null;
  /** M4-P4e: die eigene Chat-Seitenleiste geht auf (ein globaler Chat weicht). */
  onChatOpen?: () => void;
  /** M5-P5d: Popover einer Person -> Liste auf ihre Besprechungen eingrenzen. */
  onPersonFilter?: (person: PersonRef) => void;
  /** M5-P5d: Popover einer Person -> Chat ueber alle Besprechungen mit ihr. */
  onPersonAsk?: (person: PersonRef) => void;
}

export const MeetingDetail: React.FC<MeetingDetailProps> = ({
  meeting,
  onBack,
  onMeetingChange,
  jumpRequest,
  onChatOpen,
  onPersonFilter,
  onPersonAsk,
}) => {
  const { t, i18n } = useTranslation();
  const meetingId = meeting.id;
  const meetingTitle = meeting.title;
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState(meetingTitle);
  const [titleError, setTitleError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("transcript");
  const [notesView, setNotesView] = useState<NotesView>("mine");
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
  // Erhoeht sich, wenn sich die Segmente ersetzt haben koennten (Laden,
  // Neu-Transkription): die KI-Notizen lesen dann die Epoche neu.
  const [epochKey, setEpochKey] = useState(0);
  // Sprungmarken: Kanal 1 (Gegenseite) liegt in der Systemaufnahme, alles
  // andere (Mikrofon, Import als Mischkanal) im Mikrofon-/Import-Player.
  const micPlayerRef = useRef<AudioPlayerHandle>(null);
  const systemPlayerRef = useRef<AudioPlayerHandle>(null);
  const playSegment = (segment: StoredSegment) => {
    const player =
      segment.channel === 1 && systemPlayerRef.current
        ? systemPlayerRef.current
        : (micPlayerRef.current ?? systemPlayerRef.current);
    player?.playAt(segment.start_ms / 1000);
  };
  const hasAudio = Boolean(
    meeting?.mic_audio_path || meeting?.system_audio_path,
  );

  /**
   * Quelle einer KI-Notiz: ins Transkript wechseln, das Segment markieren
   * und (solange Audio da ist) ab der Stelle abspielen. Die Player liegen
   * ueber den Tabs und bleiben beim Wechsel erhalten.
   */
  const jumpToSource = (segmentIndex: number) => {
    const segment = segments.find((s) => s.segment_index === segmentIndex);
    if (!segment) return;
    setTab("transcript");
    setHighlightIndex(segmentIndex);
    if (highlightTimer.current) clearTimeout(highlightTimer.current);
    highlightTimer.current = setTimeout(
      () => setHighlightIndex(null),
      SOURCE_HIGHLIGHT_MS,
    );
    if (hasAudio) playSegment(segment);
  };

  useEffect(
    () => () => {
      if (highlightTimer.current) clearTimeout(highlightTimer.current);
    },
    [],
  );

  // Nach dem Wechsel in den Transkript-Tab steht die Zeile erst im DOM.
  useEffect(() => {
    if (highlightIndex === null || tab !== "transcript") return;
    transcriptRef.current
      ?.querySelector<HTMLElement>(`[data-segment-index="${highlightIndex}"]`)
      ?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [highlightIndex, tab]);

  // M4-P4e: Chat-Seitenleiste (Strg+J) und Belegsprung.
  const [chatOpen, setChatOpen] = useState(false);
  // M6-P6c: Follow-up-Mail
  const [followupOpen, setFollowupOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const notesRef = useRef<HTMLDivElement>(null);
  const [noteTarget, setNoteTarget] = useState<{
    selector: string;
    nonce: number;
  } | null>(null);

  const toggleChat = useCallback(() => {
    if (!chatOpen) onChatOpen?.();
    setChatOpen(!chatOpen);
  }, [chatOpen, onChatOpen]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (
        (e.ctrlKey || e.metaKey) &&
        !e.shiftKey &&
        !e.altKey &&
        e.key.toLowerCase() === "j"
      ) {
        e.preventDefault();
        toggleChat();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [toggleChat]);

  /**
   * Beleg aus dem Chat: Transkript-Stelle wie eine KI-Notizen-Quelle
   * (markieren + abspielen), Notizen-Beleg im Tab Notizen markieren.
   */
  const jumpToCitation = (citation: Citation) => {
    if (citation.source === "user_notes" || citation.source === "ai_notes") {
      setTab("notes");
      setNotesView(citation.source === "user_notes" ? "mine" : "ai");
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
    else setTab("transcript");
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
        // ready/failed einmal frisch laden, damit Badge und Player stimmen.
        if (payload.status === "ready" || payload.status === "failed") {
          void loadSegments();
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

  const main = (
    <SettingsGroup>
      <div className="px-4 py-3 space-y-3">
        <div className="flex items-center justify-between gap-2">
          <button
            type="button"
            onClick={onBack}
            className="flex items-center gap-1 text-sm text-text/70 hover:text-text cursor-pointer"
          >
            <ArrowLeft width={16} height={16} />
            {t("meetings.detail.back")}
          </button>
          <div className="flex items-center gap-2">
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
              onClick={toggleChat}
              title={t("meetings.chat.askTitle")}
              aria-pressed={chatOpen}
              aria-keyshortcuts="Control+J"
            >
              <MessageSquare width={14} height={14} aria-hidden="true" />
              {t("meetings.chat.ask")}
            </Button>
          </div>
        </div>
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

        <div className="grid grid-cols-[minmax(6rem,auto)_1fr] gap-x-4 gap-y-1 text-sm border border-mid-gray/20 rounded-md px-3 py-2">
          <span className="text-text/60">{t("meetings.meta.status")}</span>
          <span>
            <Badge
              variant={meeting.status === "ready" ? "success" : "secondary"}
            >
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
              <span className="text-text/60">
                {t("meetings.meta.duration")}
              </span>
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

        <RetranscribeControl meeting={meeting} onFinished={loadSegments} />

        <div className="flex gap-1 border-b border-mid-gray/20">
          <button
            type="button"
            onClick={() => setTab("notes")}
            className={`px-3 py-1.5 text-sm font-medium border-b-2 cursor-pointer ${
              tab === "notes"
                ? "border-logo-primary text-text"
                : "border-transparent text-text/60 hover:text-text"
            }`}
          >
            {t("meetings.notes.tab")}
          </button>
          <button
            type="button"
            onClick={() => setTab("transcript")}
            className={`px-3 py-1.5 text-sm font-medium border-b-2 cursor-pointer ${
              tab === "transcript"
                ? "border-logo-primary text-text"
                : "border-transparent text-text/60 hover:text-text"
            }`}
          >
            {t("meetings.detail.transcriptTab")}
          </button>
          <button
            type="button"
            onClick={() => setTab("minutes")}
            className={`px-3 py-1.5 text-sm font-medium border-b-2 cursor-pointer ${
              tab === "minutes"
                ? "border-logo-primary text-text"
                : "border-transparent text-text/60 hover:text-text"
            }`}
          >
            {t("meetings.detail.minutesTab")}
          </button>
        </div>

        {tab === "transcript" && segments.length > 0 && (
          <div className="flex items-center gap-2">
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
        {tab === "transcript" && transcriptError && (
          <p className="text-sm text-red-400">{transcriptError}</p>
        )}
        {tab === "transcript" && speakerNotices.length > 0 && (
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
        {tab === "notes" && (
          <div className="space-y-3" ref={notesRef}>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div
                role="group"
                aria-label={t("meetings.notes.tab")}
                className="inline-flex rounded-lg border border-mid-gray/20 p-0.5 text-sm"
              >
                {(["mine", "ai"] as const).map((view) => (
                  <button
                    key={view}
                    type="button"
                    aria-pressed={notesView === view}
                    onClick={() => setNotesView(view)}
                    className={`rounded-md px-3 py-1 cursor-pointer ${
                      notesView === view
                        ? "bg-logo-primary/20 text-text"
                        : "text-text/60 hover:text-text"
                    }`}
                  >
                    {t(`meetings.notes.view.${view}`)}
                  </button>
                ))}
              </div>
              <MeetingTemplatePicker meetingId={meetingId} />
            </div>
            {notesView === "mine" ? (
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
        )}
        {tab === "transcript" ? (
          loading ? (
            <p className="text-sm text-text/60 text-center py-3">
              {t("meetings.list.loading")}
            </p>
          ) : segments.length === 0 ? (
            <p className="text-sm text-text/60">{t("meetings.live.empty")}</p>
          ) : (
            <div
              ref={transcriptRef}
              className="space-y-2 max-h-96 overflow-y-auto"
            >
              {segments.map((segment) => (
                <div
                  key={segment.segment_index}
                  data-segment-index={segment.segment_index}
                  data-highlighted={
                    highlightIndex === segment.segment_index
                      ? "true"
                      : undefined
                  }
                  className={`flex gap-2 items-start text-sm group rounded-md px-1 -mx-1 transition-colors ${
                    highlightIndex === segment.segment_index
                      ? "bg-logo-primary/25 ring-1 ring-logo-primary/50"
                      : ""
                  }`}
                >
                  {hasAudio ? (
                    <button
                      type="button"
                      onClick={() => playSegment(segment)}
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
          )
        ) : tab === "minutes" ? (
          <MinutesView meetingId={meetingId} meetingTitle={meetingTitle} />
        ) : null}
      </div>
    </SettingsGroup>
  );

  return (
    <div className="flex flex-col gap-4 lg:flex-row lg:items-start">
      <div className="min-w-0 flex-1">{main}</div>
      {chatOpen && (
        <aside className="w-full shrink-0 lg:sticky lg:top-0 lg:w-96">
          <ChatPanel
            scope={{ kind: "meeting", meeting_id: meetingId }}
            mode="meeting"
            onClose={() => setChatOpen(false)}
            onJump={jumpToCitation}
          />
        </aside>
      )}
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
    </div>
  );
};
