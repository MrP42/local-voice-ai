import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Check,
  Combine,
  Play,
  SkipForward,
  Split,
  Square,
  TextSearch,
  Undo2,
} from "lucide-react";
import {
  commands,
  type MeetingSpeaker,
  type SpeakerMerge,
  type StoredSegment,
  type VoiceMatch,
} from "@/bindings";
import {
  absorbedInto,
  absorbedLabel,
  duplicateGroups,
  formatShare,
  formatSpeechTime,
  mergeCandidates,
  mergeDirection,
  shareRows,
  speakerKey,
  speakerSamples,
  voicePairs,
  type SpeakerSample,
} from "@/lib/meetingSpeakerShares";
import { Button } from "../../ui/Button";
import { IconButton } from "../../ui/IconButton";
import { PersonNameField } from "./PersonNameField";
import { speakerErrorText } from "./SpeakerPopover";
import type { TranscriptPlayer } from "./transcriptPlayer";

interface SpeakerSharesTableProps {
  meetingId: string;
  speakers: MeetingSpeaker[];
  segments: StoredSegment[];
  player: TranscriptPlayer;
  /** Nach dem Speichern eines Namens (Sprecher, Segmente, Dokumente neu laden). */
  onChanged: () => void;
  /** Die Stelle im Transkript zeigen und ab dort abspielen. */
  onJump: (segmentIndex: number) => void;
}

/** Touchziel 44 px: Fluent begrenzt Icon-Buttons sonst auf 40 px (lg). */
const TOUCH: React.CSSProperties = {
  width: 44,
  height: 44,
  minWidth: 44,
  maxWidth: 44,
};

/** Textknöpfe (Zusammenführen, Trennen): mindestens 44 px hoch. */
const TOUCH_TEXT: React.CSSProperties = { minHeight: 44 };
/** Längere Textknöpfe: umbrechen statt seitlich abgeschnitten. */
const TOUCH_WRAP: React.CSSProperties = {
  minHeight: 44,
  height: "auto",
  maxWidth: "100%",
  whiteSpace: "normal",
  textAlign: "start",
};

/** Läuft gerade: welcher Sprecher, welche Probe. */
interface Playing {
  key: string;
  sample: number;
}

/**
 * #242: Tabelle "Sprecher & Redeanteile" im Protokoll. Je Zeile Hörproben
 * (typische Ausschnitte über die ganze Besprechung, nacheinander; "Weiter"
 * springt zur nächsten, "Im Transkript zeigen" springt an die Stelle) und der
 * Name direkt in der Zeile. Der Name gilt besprechungsweit: Transkript,
 * Protokoll, KI-Notizen, Fragen und Export lesen ihn aus der zentralen
 * Sprecherzuordnung (Kanal, Nummer -> Name).
 *
 * #266: Zeilen, die dieselbe Person sind, lassen sich zusammenführen (Knopf
 * je Zeile; bei gleichem Namen ein Hinweis mit einem Klick). Jede
 * Zusammenführung lässt sich wieder trennen ("Trennen" unter dem Namen,
 * "Rückgängig" direkt danach).
 */
export const SpeakerSharesTable: React.FC<SpeakerSharesTableProps> = ({
  meetingId,
  speakers,
  segments,
  player,
  onChanged,
  onJump,
}) => {
  const { t, i18n } = useTranslation();
  const rows = useMemo(() => shareRows(speakers), [speakers]);
  const samples = useMemo(() => {
    const map = new Map<string, SpeakerSample[]>();
    for (const speaker of rows) {
      map.set(speakerKey(speaker), speakerSamples(segments, speaker));
    }
    return map;
  }, [rows, segments]);

  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [savedKey, setSavedKey] = useState<string | null>(null);
  /** Nächste Probe je Sprecher (nach "Weiter" und am Ende einer Probe). */
  const [cursor, setCursor] = useState<Record<string, number>>({});
  const [playing, setPlaying] = useState<Playing | null>(null);
  const playingRef = useRef<Playing | null>(null);
  const playerRef = useRef(player);
  playerRef.current = player;
  const savedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // #266: Zusammenführen und Trennen.
  const [merges, setMerges] = useState<SpeakerMerge[]>([]);
  const [mergeOpen, setMergeOpen] = useState<string | null>(null);
  const [mergeBusy, setMergeBusy] = useState(false);
  const [mergeError, setMergeError] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ text: string; undo: string[] } | null>(
    null,
  );
  const groups = useMemo(() => duplicateGroups(rows), [rows]);
  // Stimmprofile 2a: "klingen gleich" aus Stimmabdrücken dieser Besprechung.
  const [voiceMatches, setVoiceMatches] = useState<VoiceMatch[]>([]);
  const voices = useMemo(
    () => voicePairs(rows, voiceMatches),
    [rows, voiceMatches],
  );

  /** Läuft ein Zusammenführen oder Trennen (gegen Doppelklick im selben Takt). */
  const mergeBusyRef = useRef(false);
  /** Ein gerade laufendes Speichern eines Namens (Fokus wechselt zum Knopf). */
  const pendingSave = useRef<Promise<void> | null>(null);

  useEffect(() => {
    let current = true;
    void (async () => {
      try {
        const result = await commands.meetingSpeakerMerges(meetingId);
        if (current && result.status === "ok")
          setMerges(Array.isArray(result.data) ? result.data : []);
      } catch {
        // Ohne Liste gibt es nur kein "Trennen"; Zusammenführen geht weiter.
      }
    })();
    return () => {
      current = false;
    };
  }, [meetingId, speakers]);

  // Andere Besprechung: alte Vorschläge sofort weg (nicht bei jeder Änderung,
  // sonst flackert der Hinweis bei jedem Umbenennen).
  useEffect(() => setVoiceMatches([]), [meetingId]);

  useEffect(() => {
    let current = true;
    void (async () => {
      try {
        const result = await commands.meetingVoiceMatches(meetingId);
        if (
          current &&
          result.status === "ok" &&
          result.data?.state === "ready" &&
          Array.isArray(result.data.matches)
        )
          setVoiceMatches(result.data.matches);
      } catch {
        // Ohne Modell oder Audio gibt es nur keinen Vorschlag.
      }
    })();
    return () => {
      current = false;
    };
  }, [meetingId, speakers]);

  /** "Verschiedene Personen": Vorschlag sofort weg, dann merken. */
  const dismissVoice = async (match: {
    from: MeetingSpeaker;
    into: MeetingSpeaker;
  }) => {
    const { channel } = match.from;
    const a = Math.min(match.from.speaker_index, match.into.speaker_index);
    const b = Math.max(match.from.speaker_index, match.into.speaker_index);
    const same = (m: VoiceMatch) =>
      m.channel === channel && m.a === a && m.b === b;
    const removed = voiceMatches.filter(same);
    setVoiceMatches((old) => old.filter((m) => !same(m)));
    let ok = false;
    try {
      const result = await commands.meetingVoiceMatchDismiss(
        meetingId,
        channel,
        a,
        b,
      );
      ok = result.status === "ok";
    } catch {
      ok = false;
    }
    if (!ok) {
      // Nicht gemerkt: Vorschlag wieder zeigen, statt ihn still zu verlieren.
      setVoiceMatches((old) => [...old, ...removed]);
      setMergeError(speakerErrorText("generic", t));
    }
  };

  // Andere Besprechung: kein "Rückgängig" und keine offene Auswahl von vorher.
  useEffect(() => {
    setNotice(null);
    setMergeOpen(null);
    setMergeError(null);
  }, [meetingId]);

  // Geöffnete Auswahl ganz zeigen (schmale Fenster, Hinweis am unteren Rand).
  useEffect(() => {
    if (!mergeOpen) return;
    const panel = document.getElementById(
      `speaker-merge-${mergeOpen.replace(":", "-")}`,
    );
    panel?.scrollIntoView?.({ block: "center", behavior: "auto" });
  }, [mergeOpen]);

  // Ansicht verlassen: keine Probe läuft weiter.
  useEffect(
    () => () => {
      if (playingRef.current) {
        playingRef.current = null;
        playerRef.current.stop();
      }
      if (savedTimer.current) clearTimeout(savedTimer.current);
    },
    [],
  );

  const setRunning = (next: Playing | null) => {
    playingRef.current = next;
    setPlaying(next);
  };

  /** Spielt Probe `index` und danach die folgenden, bis die letzte endet. */
  const playFrom = (key: string, index: number) => {
    const list = samples.get(key) ?? [];
    const sample = list[index];
    if (!sample) {
      setRunning(null);
      return;
    }
    const run = { key, sample: index };
    setRunning(run);
    setCursor((old) => ({ ...old, [key]: index }));
    playerRef.current.playRange(
      sample.start_ms,
      sample.end_ms,
      sample.channel,
      (completed) => {
        const current = playingRef.current;
        // Abgelöst (andere Probe, Stopp, Weiter): nichts tun.
        if (!current || current.key !== key || current.sample !== index) return;
        // Von außen angehalten (Pause, Sprung im Player): Kette beenden.
        if (!completed) {
          setRunning(null);
          return;
        }
        if (index + 1 < list.length) {
          playFrom(key, index + 1);
        } else {
          setRunning(null);
          setCursor((old) => ({ ...old, [key]: 0 }));
        }
      },
    );
  };

  const togglePlay = (key: string) => {
    if (playingRef.current?.key === key) {
      setRunning(null);
      playerRef.current.stop();
      return;
    }
    playFrom(key, cursor[key] ?? 0);
  };

  const next = (key: string) => {
    const total = samples.get(key)?.length ?? 0;
    if (total === 0) return;
    const current =
      playingRef.current?.key === key
        ? playingRef.current.sample
        : (cursor[key] ?? 0);
    playFrom(key, (current + 1) % total);
  };

  const jump = (key: string) => {
    const list = samples.get(key) ?? [];
    const index =
      playingRef.current?.key === key
        ? playingRef.current.sample
        : (cursor[key] ?? 0);
    const sample = list[index] ?? list[0];
    if (!sample) return;
    if (playingRef.current) {
      setRunning(null);
      playerRef.current.stop();
    }
    onJump(sample.segment_index);
  };

  const stopSamples = () => {
    if (playingRef.current) {
      setRunning(null);
      playerRef.current.stop();
    }
  };

  /** Name nach dem Zusammenführen: der des Ziels, sonst der mitgenommene. */
  const mergedName = (
    pairs: { from: MeetingSpeaker; into: MeetingSpeaker }[],
  ) =>
    pairs[0].into.display_name ??
    pairs.find((p) => p.from.display_name)?.from.display_name ??
    pairs[0].into.label;

  /** Führt nacheinander zusammen; was gelang, lässt sich rückgängig machen. */
  const runMerges = async (
    pairs: { from: MeetingSpeaker; into: MeetingSpeaker }[],
  ) => {
    if (pairs.length === 0 || mergeBusyRef.current) return;
    mergeBusyRef.current = true;
    stopSamples();
    setMergeBusy(true);
    setMergeError(null);
    setNotice(null);
    const ids: string[] = [];
    try {
      await pendingSave.current;
      for (const { from, into } of pairs) {
        const result = await commands.meetingSpeakerMerge(
          meetingId,
          from.channel,
          from.speaker_index,
          into.speaker_index,
        );
        if (result.status === "error") {
          setMergeError(speakerErrorText(result.error, t));
          break;
        }
        ids.push(result.data);
      }
    } catch {
      setMergeError(speakerErrorText("generic", t));
    } finally {
      mergeBusyRef.current = false;
      setMergeBusy(false);
    }
    if (ids.length === 0) return;
    setNotice({
      text: t("meetings.speakers.merge.done", {
        count: ids.length + 1,
        name: mergedName(pairs),
      }),
      undo: ids,
    });
    setMergeOpen(null);
    onChanged();
  };

  /** Trennt (neueste zuerst); bricht beim ersten Fehler ab. */
  const runUnmerge = async (ids: string[]) => {
    if (ids.length === 0 || mergeBusyRef.current) return;
    mergeBusyRef.current = true;
    stopSamples();
    setMergeBusy(true);
    setMergeError(null);
    let done = 0;
    try {
      await pendingSave.current;
      for (const id of [...ids].reverse()) {
        const result = await commands.meetingSpeakerUnmerge(meetingId, id);
        if (result.status === "error") {
          setMergeError(speakerErrorText(result.error, t));
          break;
        }
        done++;
      }
    } catch {
      setMergeError(speakerErrorText("generic", t));
    } finally {
      mergeBusyRef.current = false;
      setMergeBusy(false);
    }
    if (done === 0) return;
    setNotice({ text: t("meetings.speakers.merge.split"), undo: [] });
    onChanged();
  };

  /** Alle Zeilen einer Gruppe gehen im ersten benannten Sprecher (nicht
   *  "Aufnahme") mit der längsten Redezeit auf. */
  const mergeGroup = (group: MeetingSpeaker[]) => {
    const into = group.find((s) => s.speaker_index !== 0) ?? group[0];
    return runMerges(
      group.filter((s) => s !== into).map((from) => ({ from, into })),
    );
  };

  const save = (speaker: MeetingSpeaker, value: string) => {
    const run = saveNow(speaker, value);
    pendingSave.current = run;
    return run.finally(() => {
      if (pendingSave.current === run) pendingSave.current = null;
    });
  };

  const saveNow = async (speaker: MeetingSpeaker, value: string) => {
    const key = speakerKey(speaker);
    const trimmed = value.trim();
    if (trimmed === (speaker.display_name ?? "")) {
      setDrafts((old) => {
        const copy = { ...old };
        delete copy[key];
        return copy;
      });
      return;
    }
    setBusy(key);
    setErrors((old) => {
      const copy = { ...old };
      delete copy[key];
      return copy;
    });
    try {
      const result = await commands.meetingSpeakerRename(
        meetingId,
        speaker.channel,
        speaker.speaker_index,
        trimmed === "" ? null : trimmed,
      );
      if (result.status === "error") {
        setErrors((old) => ({
          ...old,
          [key]: speakerErrorText(result.error, t),
        }));
        return;
      }
    } catch {
      setErrors((old) => ({ ...old, [key]: speakerErrorText("generic", t) }));
      return;
    } finally {
      setBusy(null);
    }
    setDrafts((old) => {
      const copy = { ...old };
      delete copy[key];
      return copy;
    });
    setSavedKey(key);
    if (savedTimer.current) clearTimeout(savedTimer.current);
    savedTimer.current = setTimeout(() => setSavedKey(null), 2500);
    onChanged();
  };

  const cancel = (key: string) =>
    setDrafts((old) => {
      const copy = { ...old };
      delete copy[key];
      return copy;
    });

  if (rows.length === 0) return null;

  return (
    <div className="@container mt-2" data-speaker-shares="">
      <p className="mb-2 text-xs text-text/60">
        {player.canSeek
          ? t("meetings.speakers.table.hint")
          : t("meetings.speakers.table.hintNoAudio")}
      </p>
      {groups.map((group) => (
        <div
          key={`${group[0].channel}:${group[0].speaker_index}`}
          data-testid="speaker-duplicate-hint"
          className="mb-2 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md bg-logo-primary/10 px-3 py-1.5 text-sm"
        >
          <span className="min-w-[12rem] flex-1">
            {t("meetings.speakers.merge.duplicate", {
              name: group[0].display_name,
              count: group.length,
            })}
          </span>
          <Button
            variant="secondary"
            size="md"
            style={TOUCH_WRAP}
            disabled={mergeBusy}
            data-testid="speaker-duplicate-merge"
            onClick={() => void mergeGroup(group)}
          >
            <Combine width={16} height={16} aria-hidden="true" />
            {t("meetings.speakers.merge.duplicateAction")}
          </Button>
        </div>
      ))}
      {voices.map((match) => (
        <div
          key={`voice:${match.from.channel}:${match.from.speaker_index}:${match.into.speaker_index}`}
          data-testid="speaker-voice-hint"
          className="mb-2 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md bg-logo-primary/10 px-3 py-1.5 text-sm"
        >
          <span className="min-w-[12rem] flex-1">
            {t("meetings.speakers.merge.voice", {
              a: match.from.label,
              b: match.into.label,
              score: match.score,
            })}
            <span className="block text-xs text-text/60">
              {t("meetings.speakers.merge.voiceNote")}
            </span>
          </span>
          <span className="flex flex-wrap items-center gap-2">
            <Button
              variant="secondary"
              size="md"
              style={TOUCH_WRAP}
              disabled={mergeBusy}
              data-testid="speaker-voice-merge"
              onClick={() => void runMerges([match])}
            >
              <Combine width={16} height={16} aria-hidden="true" />
              {t("meetings.speakers.merge.voiceAction")}
            </Button>
            <Button
              variant="ghost"
              size="md"
              style={TOUCH_WRAP}
              disabled={mergeBusy}
              data-testid="speaker-voice-dismiss"
              onClick={() => void dismissVoice(match)}
            >
              {t("meetings.speakers.merge.voiceDismiss")}
            </Button>
          </span>
        </div>
      ))}
      {notice && (
        <div
          role="status"
          data-testid="speaker-merge-notice"
          className="mb-2 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md border border-mid-gray/30 px-3 py-1.5 text-sm"
        >
          <span className="min-w-[12rem] flex-1">{notice.text}</span>
          {notice.undo.length > 0 && (
            <Button
              variant="ghost"
              size="md"
              style={TOUCH_TEXT}
              disabled={mergeBusy}
              data-testid="speaker-merge-undo"
              onClick={() => void runUnmerge(notice.undo)}
            >
              <Undo2 width={16} height={16} aria-hidden="true" />
              {t("meetings.speakers.merge.undo")}
            </Button>
          )}
        </div>
      )}
      {mergeError && (
        <p
          role="alert"
          className="mb-2 text-xs text-red-400"
          data-testid="speaker-merge-error"
        >
          {mergeError}
        </p>
      )}
      <div role="table" aria-label={t("meetings.speakers.table.label")}>
        <div
          role="row"
          className="hidden border-b border-mid-gray/30 pb-1 text-sm font-semibold @xl:grid @xl:grid-cols-[minmax(0,1fr)_11.5rem_5rem_5rem] @xl:gap-3"
        >
          <span role="columnheader">
            {t("meetings.speakers.table.speaker")}
          </span>
          <span role="columnheader">{t("meetings.speakers.table.listen")}</span>
          <span role="columnheader">{t("meetings.speakers.table.time")}</span>
          <span role="columnheader">{t("meetings.speakers.table.share")}</span>
        </div>
        {rows.map((speaker) => {
          const key = speakerKey(speaker);
          const list = samples.get(key) ?? [];
          const isPlaying = playing?.key === key;
          const shownSample = isPlaying ? playing.sample : (cursor[key] ?? 0);
          const draft = drafts[key] ?? speaker.display_name ?? "";
          const time = formatSpeechTime(speaker.speech_ms);
          const share = formatShare(speaker.share_pct, i18n.language);
          const label = speaker.label;
          const candidates = mergeCandidates(rows, speaker);
          const absorbed = absorbedInto(merges, speaker);
          const panelId = `speaker-merge-${key.replace(":", "-")}`;
          const panelOpen = mergeOpen === key;
          return (
            <div
              key={key}
              role="row"
              data-testid="speaker-share-row"
              data-speaker={key}
              data-copy-name={label}
              data-copy-time={time}
              data-copy-share={share}
              className="grid grid-cols-[11.5rem_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1 border-b border-mid-gray/20 py-2 @xl:grid-cols-[minmax(0,1fr)_11.5rem_5rem_5rem]"
            >
              <div role="cell" className="col-span-3 min-w-0 @xl:col-span-1">
                <div className="flex min-w-0 items-center gap-1">
                  <form
                    onSubmit={(e) => {
                      e.preventDefault();
                      void save(speaker, draft);
                    }}
                    onBlur={(e) => {
                      // Fokus bleibt in der Zeile (Treffer der Personenliste):
                      // noch nicht speichern.
                      if (
                        e.currentTarget.contains(e.relatedTarget as Node | null)
                      )
                        return;
                      if (drafts[key] !== undefined) void save(speaker, draft);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Escape" && drafts[key] !== undefined) {
                        e.stopPropagation();
                        cancel(key);
                      }
                    }}
                    className="flex min-w-0 flex-1 items-center gap-2"
                  >
                    <PersonNameField
                      value={draft}
                      onChange={(value) =>
                        setDrafts((old) => ({ ...old, [key]: value }))
                      }
                      onPick={(person) => {
                        setDrafts((old) => ({ ...old, [key]: person.name }));
                        void save(speaker, person.name);
                      }}
                      placeholder={speaker.default_label}
                      ariaLabel={t("meetings.speakers.table.nameFor", {
                        label: speaker.default_label,
                      })}
                      disabled={busy === key}
                      inputTestId="speaker-share-name"
                      inputClassName="[&>input]:min-h-[44px]"
                      exclude={
                        speaker.display_name ? [speaker.display_name] : []
                      }
                      className="min-w-0 flex-1"
                    />
                    {savedKey === key && (
                      <Check
                        width={16}
                        height={16}
                        aria-label={t("meetings.speakers.table.saved")}
                        className="shrink-0 text-logo-primary"
                        data-testid="speaker-share-saved"
                      />
                    )}
                  </form>
                  <IconButton
                    size="lg"
                    variant={panelOpen ? "secondary" : "ghost"}
                    style={TOUCH}
                    disabled={candidates.length === 0 || mergeBusy}
                    aria-expanded={panelOpen}
                    aria-controls={panelOpen ? panelId : undefined}
                    icon={<Combine width={16} height={16} aria-hidden="true" />}
                    label={
                      candidates.length === 0
                        ? t("meetings.speakers.merge.none")
                        : t("meetings.speakers.merge.openFor", { label })
                    }
                    data-testid="speaker-share-merge"
                    onClick={() => {
                      setMergeError(null);
                      setMergeOpen(panelOpen ? null : key);
                    }}
                  />
                </div>
                {absorbed.length > 0 && (
                  <div
                    className="mt-1 flex flex-wrap items-center gap-x-2 text-xs text-text/60"
                    data-testid="speaker-absorbed"
                  >
                    <span>{t("meetings.speakers.merge.contains")}</span>
                    {absorbed.map((m) => {
                      const shown = absorbedLabel(m, label);
                      return (
                        <span
                          key={m.id}
                          className="inline-flex items-center gap-1"
                          data-testid="speaker-absorbed-item"
                        >
                          <span className="text-text/80">{shown}</span>
                          <Button
                            variant="ghost"
                            size="md"
                            style={TOUCH_TEXT}
                            disabled={!m.undoable || mergeBusy}
                            title={
                              m.undoable
                                ? t("meetings.speakers.merge.splitFor", {
                                    label: shown,
                                  })
                                : t("meetings.speakers.merge.blocked")
                            }
                            aria-label={t("meetings.speakers.merge.splitFor", {
                              label: shown,
                            })}
                            data-testid="speaker-absorbed-split"
                            onClick={() => void runUnmerge([m.id])}
                          >
                            <Split width={14} height={14} aria-hidden="true" />
                            {t("meetings.speakers.merge.splitAction")}
                          </Button>
                        </span>
                      );
                    })}
                  </div>
                )}
                {errors[key] && (
                  <p role="alert" className="mt-1 text-xs text-red-400">
                    {errors[key]}
                  </p>
                )}
              </div>
              <div role="cell" className="flex items-center gap-1">
                <IconButton
                  size="lg"
                  variant={isPlaying ? "secondary" : "ghost"}
                  style={TOUCH}
                  disabled={!player.canSeek || list.length === 0}
                  aria-pressed={isPlaying}
                  icon={
                    isPlaying ? (
                      <Square
                        width={16}
                        height={16}
                        fill="currentColor"
                        aria-hidden="true"
                      />
                    ) : (
                      <Play width={16} height={16} aria-hidden="true" />
                    )
                  }
                  label={
                    isPlaying
                      ? t("meetings.speakers.table.stopFor", { label })
                      : t("meetings.speakers.table.playFor", { label })
                  }
                  data-testid="speaker-share-play"
                  onClick={() => togglePlay(key)}
                />
                <IconButton
                  size="lg"
                  style={TOUCH}
                  disabled={!player.canSeek || list.length < 2}
                  icon={
                    <SkipForward width={16} height={16} aria-hidden="true" />
                  }
                  label={t("meetings.speakers.table.nextFor", { label })}
                  data-testid="speaker-share-next"
                  onClick={() => next(key)}
                />
                <IconButton
                  size="lg"
                  style={TOUCH}
                  disabled={list.length === 0}
                  icon={
                    <TextSearch width={16} height={16} aria-hidden="true" />
                  }
                  label={t("meetings.speakers.table.jumpFor", { label })}
                  data-testid="speaker-share-jump"
                  onClick={() => jump(key)}
                />
                <span
                  className="w-9 text-center text-xs tabular-nums text-text/60"
                  aria-live="polite"
                  data-testid="speaker-share-counter"
                >
                  {list.length > 0
                    ? t("meetings.speakers.table.sampleOf", {
                        n: Math.min(shownSample + 1, list.length),
                        total: list.length,
                      })
                    : ""}
                </span>
              </div>
              <div
                role="cell"
                className="text-end text-sm whitespace-nowrap tabular-nums @xl:text-start"
                data-testid="speaker-share-time"
              >
                <span className="sr-only @xl:hidden">
                  {t("meetings.speakers.table.time")}:{" "}
                </span>
                {time}
              </div>
              <div
                role="cell"
                className="text-end text-sm tabular-nums @xl:text-start"
                data-testid="speaker-share-pct"
              >
                {share}
              </div>
              {panelOpen && (
                <div
                  role="cell"
                  id={panelId}
                  data-testid="speaker-merge-panel"
                  className="col-span-full rounded-md border border-mid-gray/30 p-2"
                >
                  <p className="mb-1 text-sm font-semibold">
                    {t("meetings.speakers.merge.question", { label })}
                  </p>
                  <p className="mb-2 text-xs text-text/60">
                    {t("meetings.speakers.merge.rule")}
                  </p>
                  <div className="flex flex-wrap gap-2">
                    {candidates.map((other) => (
                      <Button
                        key={speakerKey(other)}
                        variant="secondary"
                        size="md"
                        style={TOUCH_TEXT}
                        disabled={mergeBusy}
                        data-testid="speaker-merge-target"
                        data-speaker={speakerKey(other)}
                        onClick={() =>
                          void runMerges([mergeDirection(speaker, other)])
                        }
                      >
                        {other.label}
                        <span className="text-xs tabular-nums text-text/60">
                          {formatSpeechTime(other.speech_ms)}
                        </span>
                      </Button>
                    ))}
                    <Button
                      variant="ghost"
                      size="md"
                      style={TOUCH_TEXT}
                      onClick={() => setMergeOpen(null)}
                    >
                      {t("meetings.speakers.merge.cancel")}
                    </Button>
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
};

/**
 * Für die Zwischenablage: die Tabelle als einfache HTML-Tabelle wie im
 * gespeicherten Protokoll (ohne Felder und Knöpfe), mit den aktuellen Namen.
 */
export function staticSharesTable(
  live: Element,
  headers: { speaker: string; time: string; share: string },
): HTMLTableElement {
  const doc = live.ownerDocument;
  const table = doc.createElement("table");
  const head = table.createTHead().insertRow();
  for (const text of [headers.speaker, headers.time, headers.share]) {
    const th = doc.createElement("th");
    th.textContent = text;
    head.appendChild(th);
  }
  const body = table.createTBody();
  live.querySelectorAll("[data-copy-name]").forEach((row) => {
    const tr = body.insertRow();
    for (const attr of [
      "data-copy-name",
      "data-copy-time",
      "data-copy-share",
    ]) {
      tr.insertCell().textContent = row.getAttribute(attr) ?? "";
    }
  });
  return table;
}
