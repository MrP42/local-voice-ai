import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft, Check, Download, Pencil, X } from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { commands, type Meeting, type StoredSegment } from "@/bindings";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { Button } from "../../ui/Button";
import { Textarea } from "../../ui/Textarea";
import { AudioPlayer, AudioPlayerGroup } from "../../ui/AudioPlayer";
import Badge from "../../ui/Badge";
import { MinutesView } from "./MinutesView";
import { RetranscribeControl } from "./RetranscribeControl";
import { Input } from "../../ui/Input";
import { translateMeetingError } from "./meetingErrors";

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

type Tab = "transcript" | "minutes";

// Windows paths use backslashes; the old class `[\/]` matched only the
// forward slash, so a C:\... path came back whole.
const fileBaseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

interface MeetingDetailProps {
  meeting: Meeting;
  onBack: () => void;
  /** Propagates a title change back to the list, which owns the record. */
  onMeetingChange: (meeting: Meeting) => void;
}

export const MeetingDetail: React.FC<MeetingDetailProps> = ({
  meeting,
  onBack,
  onMeetingChange,
}) => {
  const { t, i18n } = useTranslation();
  const meetingId = meeting.id;
  const meetingTitle = meeting.title;
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState(meetingTitle);
  const [titleError, setTitleError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("transcript");
  const [segments, setSegments] = useState<StoredSegment[]>([]);
  const [loading, setLoading] = useState(true);
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [copied, setCopied] = useState<"meta" | "plain" | null>(null);
  const [transcriptError, setTranscriptError] = useState<string | null>(null);
  const [editText, setEditText] = useState("");
  const [saving, setSaving] = useState(false);

  const loadSegments = useCallback(async () => {
    setLoading(true);
    const result = await commands.meetingsGetSegments(meetingId);
    setLoading(false);
    if (result.status === "ok") {
      // Segments come back in segment_index order, which interleaves
      // channels for a live-recorded meeting — always sort by start_ms.
      setSegments([...result.data].sort((a, b) => a.start_ms - b.start_ms));
    }
  }, [meetingId]);

  useEffect(() => {
    void loadSegments();
  }, [loadSegments]);

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
  const showChannels = new Set(segments.map((s) => s.channel)).size > 1;

  /** `withMeta` false liefert den blanken Text — ohne Zeitstempel, ohne
   *  Quelle, mit Leerzeile zwischen den Abschnitten, damit er sich als
   *  Fliesstext weiterverwenden laesst. */
  const transcriptText = (withMeta: boolean) =>
    segments
      .map((s) => {
        if (!withMeta) return s.text;
        const who = showChannels ? `${t(channelLabelKey(s.channel))} ` : ``;
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
      .map(
        (s) =>
          `**${t(channelLabelKey(s.channel))} [${formatMmSs(s.start_ms)}]:** ${s.text}`,
      )
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

  return (
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
        {tab === "transcript" ? (
          loading ? (
            <p className="text-sm text-text/60 text-center py-3">
              {t("meetings.list.loading")}
            </p>
          ) : segments.length === 0 ? (
            <p className="text-sm text-text/60">{t("meetings.live.empty")}</p>
          ) : (
            <div className="space-y-2 max-h-96 overflow-y-auto">
              {segments.map((segment) => (
                <div
                  key={segment.segment_index}
                  className="flex gap-2 items-start text-sm group"
                >
                  <span className="text-xs text-text/40 w-10 shrink-0 pt-0.5">
                    {formatMmSs(segment.start_ms)}
                  </span>
                  {showChannels && (
                    <span className="text-xs text-text/50 w-16 shrink-0 pt-0.5">
                      {t(channelLabelKey(segment.channel))}
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
        ) : (
          <MinutesView meetingId={meetingId} meetingTitle={meetingTitle} />
        )}
      </div>
    </SettingsGroup>
  );
};
