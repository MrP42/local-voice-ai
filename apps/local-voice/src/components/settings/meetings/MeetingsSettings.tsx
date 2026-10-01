import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { PanelLeftOpen } from "lucide-react";
import { toast } from "sonner";
import { PageShell } from "../../ui/PageShell";
import { usePersistentState } from "../../../hooks/usePersistentState";
import { useRecordingActive } from "../../../hooks/useRecordingActive";
import {
  commands,
  events,
  type BriefInfo,
  type Citation,
  type EntrySource,
  type Meeting,
  type ProjectMinutes,
  type ScopeFilter,
  type SourceRecording,
} from "@/bindings";
import { RecorderCard } from "./RecorderCard";
import { MeetingList } from "./MeetingList";
import { MeetingDetail } from "./MeetingDetail";
import { YoutubeLinkHost } from "./youtube/YoutubeLinkHost";
import { RecWorkspace } from "./RecWorkspace";
import {
  CENTER_TAB_KEY,
  LOWER_TAB_KEY,
  centerFromLegacy,
  isCenterTab,
  isLowerTab,
  lowerFromLegacy,
  readLegacyTab,
  type CenterTab,
  type LowerTab,
  type NotesTab,
} from "@/lib/meetingTabs";
import { useProjects } from "./projects/useProjects";
import { useMeetingDrag } from "./projects/useMeetingDrag";
import { ProjectsRail } from "./projects/ProjectsRail";
import { DragGhost } from "./projects/DragGhost";
import { useRecLayout } from "./useRecLayout";
import { ChatPanel } from "./chat/ChatPanel";
import { EMPTY_SCOPE } from "./chat/ScopeChips";
import type { PersonRef } from "./people/PersonPopover";
import { useImportDrop, useMeetingImport } from "./useMeetingImport";
import { useSlidesBackground } from "./slides/useSlidesBackground";
import { useSelectedProject } from "./projects/selectedProject";
import { findMeeting } from "./findMeeting";
import { isEmptyEntry, requestStartDialog } from "./emptyEntry";
import { notifyMeetingsChanged, useOpenSegment } from "@/lib/meetingsBus";
import { openYoutubeLinkDialog } from "./youtube/linkBus";
import { useMeetingProgress } from "@/hooks/useMeetingJobs";
import {
  notifyProjectMinutesChanged,
  projectMinutesErrorCode,
  projectMinutesErrorDetail,
  projectMinutesJobKey,
  sourceToCitation,
} from "@/lib/projectMinutes";
import {
  ProjectMinutesRun,
  ProjectMinutesView,
  type OpenProjectMinutes,
} from "./projectMinutes/ProjectMinutesView";

type JumpRequest = { citation: Citation; nonce: number };

/** Was der Chat in der Seitenleiste beim Oeffnen eines Briefs tun soll (M5-P5e). */
type BriefRun = {
  recipe: {
    id: string;
    nonce: number;
    values: Record<string, string>;
    display: string;
  } | null;
  thread: { id: string; nonce: number } | null;
};

/** Eine Besprechung, die es in der Liste (noch) nicht gibt, aber gerade aufgenommen wird. */
const liveStub = (id: string, title: string): Meeting => {
  const now = Math.floor(Date.now() / 1000);
  return {
    id,
    title,
    status: "recording",
    source: "recording",
    started_at: now,
    ended_at: null,
    language: null,
    mic_audio_path: null,
    system_audio_path: null,
    duration_ms: null,
    consent_confirmed_at: now,
    audio_retention_until: null,
    created_at: now,
    source_path: null,
    description: null,
    deleted_at: null,
  };
};

/** Wartezeiten vor den Versuchen, die laufende Besprechung in der Liste zu finden. */
const FIND_RECORDING_WAITS_MS = [0, 250, 700];

export const MeetingsSettings: React.FC = () => {
  const { t, i18n } = useTranslation();
  const [selected, setSelected] = useState<Meeting | null>(null);
  const selectedRef = useRef<Meeting | null>(null);
  selectedRef.current = selected;
  // G1: ein eben angelegter Eintrag: sein Titel ist gleich umbenennbar.
  const [autoRenameId, setAutoRenameId] = useState<string | null>(null);
  // Die gewaehlte Besprechung bleibt ueber Neuladen, Seitenwechsel und
  // Neustart (nur die ID; der Datensatz kommt frisch aus dem Backend).
  const [selectedId, setSelectedId] = usePersistentState<string>(
    "meetings.selected",
    "",
  );
  // G3 (#70): ein Projekt-Protokoll (oder der laufende Lauf) in der Arbeitsflaeche.
  // Es verdraengt die Detailansicht und weicht, sobald eine Besprechung gewaehlt wird.
  const [openPm, setOpenPm] = useState<OpenProjectMinutes | null>(null);
  const [pmRunError, setPmRunError] = useState<{
    folderId: string;
    text: string;
  } | null>(null);
  const progressMap = useMeetingProgress();
  // D4: Hinweis am Ende jeder Folienerkennung, Start der beim Import vorgemerkten.
  useSlidesBackground();
  const select = useCallback(
    (meeting: Meeting | null) => {
      setSelected(meeting);
      setSelectedId(meeting?.id ?? "");
      if (meeting) setOpenPm(null);
    },
    [setSelectedId],
  );
  // Chat ueber viele Besprechungen: steht im Reiter "Fragen" der rechten
  // Spalte und bleibt stehen, wenn ein Beleg die Besprechung daneben oeffnet.
  const [globalFilter, setGlobalFilter] = useState<ScopeFilter | null>(null);
  const [jump, setJump] = useState<JumpRequest | null>(null);
  // M5-P5d: Personenfilter der Liste; M5-P5e: Brief zu einem Termin.
  const [personFilter, setPersonFilter] = useState<PersonRef | null>(null);
  const [briefRun, setBriefRun] = useState<BriefRun | null>(null);
  // G4: Mitte = Transkript / Protokoll, rechts unten = Notizen / KI-Notizen /
  // Fragen. Die alten Schluessel (`meetings.midTab`, `meetings.rightTab`) liefern
  // beim ersten Start die Anfangswerte.
  const [centerTab, setCenterTab] = usePersistentState<CenterTab>(
    CENTER_TAB_KEY,
    centerFromLegacy(readLegacyTab("meetings.midTab")),
    isCenterTab,
  );
  const [rightTab, setRightTab] = usePersistentState<LowerTab>(
    LOWER_TAB_KEY,
    lowerFromLegacy(
      readLegacyTab("meetings.midTab"),
      readLegacyTab("meetings.rightTab"),
    ),
    isLowerTab,
  );
  // Der Notizen-Reiter, auf den das Schliessen der Fragen zurueckfuehrt; er
  // bleibt waehrend der Fragen eingehaengt (Notizblock, ungespeicherte Zeichen).
  const notesTabRef = useRef<NotesTab>(rightTab === "ai" ? "ai" : "notes");
  if (rightTab !== "chat") notesTabRef.current = rightTab;
  const notesTab = notesTabRef.current;
  const recording = useRecordingActive();
  const layout = useRecLayout(recording.active);
  // Projekte (= Ordner) und das Ziehen von Besprechungen darauf.
  const projects = useProjects();
  const drag = useMeetingDrag((meeting, target, additive) => {
    void projects.assign(
      meeting,
      projects.selection,
      target,
      additive ? "add" : "move",
    );
  });
  const [projectsActionsEl, setProjectsActionsEl] =
    useState<HTMLDivElement | null>(null);

  // Die Bereiche, in die die Detailansicht ihre Teile legt.
  const [contentEl, setContentEl] = useState<HTMLDivElement | null>(null);
  const [controlsEl, setControlsEl] = useState<HTMLDivElement | null>(null);
  const [notesEl, setNotesEl] = useState<HTMLDivElement | null>(null);

  // Gemerkte Besprechung beim Start laden; gibt es sie nicht mehr, vergessen.
  // Hat inzwischen etwas anderes die Auswahl uebernommen (eine laufende
  // Aufnahme), bleibt es dabei.
  useEffect(() => {
    if (!selectedId || selected) return;
    let cancelled = false;
    void findMeeting(selectedId).then((meeting) => {
      if (cancelled) return;
      if (meeting) setSelected((prev) => prev ?? meeting);
      else setSelectedId("");
    });
    return () => {
      cancelled = true;
    };
    // Nur beim Einhaengen: spaetere Auswahlen setzen `selected` selbst.
  }, []);

  // Die laufende Aufnahme ist die gewaehlte Besprechung: Notizblock in der
  // Mitte, Live-Transkript rechts, Fragen im Reiter. Geschieht einmal je
  // Aufnahme (auch wenn die Seite mitten in einer Aufnahme geoeffnet wird);
  // danach darf der Nutzer eine andere Besprechung ansehen. Der Start ueber die
  // Aufnahmekarte meldet die Besprechung selbst (`startedHere`); ueber andere
  // Wege (Kalender, Hinweisfenster) kommt sie aus der Liste.
  const handledRecording = useRef<string | null>(null);
  // Die Suche laeuft ueber mehrere Versuche weiter, auch wenn sich `t` oder
  // eine Abhaengigkeit aendert (sie wuerde sonst abgebrochen und nie wiederholt,
  // weil die Aufnahme schon als bearbeitet gilt); nur das Ausblenden der Seite beendet sie.
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const titleFallback = useRef(t("meetings.record.titlePlaceholder"));
  titleFallback.current = t("meetings.record.titlePlaceholder");
  const startedHere = useCallback(
    (meeting: Meeting) => {
      handledRecording.current = meeting.id;
      select(meeting);
      setCenterTab("transcript");
      setRightTab("notes");
    },
    [select, setCenterTab, setRightTab],
  );
  useEffect(() => {
    const id = recording.active ? recording.meetingId : null;
    if (!id || handledRecording.current === id) return;
    handledRecording.current = id;
    void (async () => {
      let meeting: Meeting | null = null;
      for (const wait of FIND_RECORDING_WAITS_MS) {
        if (wait) await new Promise((resolve) => setTimeout(resolve, wait));
        if (!mounted.current) return;
        meeting = await findMeeting(id);
        if (meeting) break;
      }
      if (!mounted.current) return;
      // Ohne Eintrag in der Liste (Datenbank gerade beschaeftigt) trotzdem
      // zeigen: die Aufnahme laeuft, die Notizen muessen tippbar sein.
      select(meeting ?? liveStub(id, titleFallback.current));
      setCenterTab("transcript");
      setRightTab("notes");
    })();
  }, [
    recording.active,
    recording.meetingId,
    select,
    setCenterTab,
    setRightTab,
  ]);

  // Import: Symbol und Ablage auf der Arbeitsflaeche fuehren in EINEN Weg.
  // Eine laufende Aufnahme behaelt den Fokus; der Import laeuft links mit
  // Fortschritt weiter.
  const recordingActiveRef = useRef(recording.active);
  recordingActiveRef.current = recording.active;
  // G1 (#70): ist ein LEERER Eintrag gewaehlt, fuellen Aufnahme, Datei und
  // Link ihn statt eine neue Besprechung anzulegen.
  const emptyTarget =
    selected !== null && isEmptyEntry(selected) ? selected : null;
  const importer = useMeetingImport(
    useCallback(
      (meeting: Meeting) => {
        // Ein Ziel bleibt gewaehlt (und zeigt jetzt den Import), auch waehrend einer
        // Aufnahme: es ist ja die Besprechung, die der Nutzer ansieht.
        const isTarget = selectedRef.current?.id === meeting.id;
        if (recordingActiveRef.current && !isTarget) return;
        select(meeting);
        setCenterTab("transcript");
      },
      [select, setCenterTab],
    ),
    emptyTarget,
  );
  const dropOver = useImportDrop(importer.ask);

  /**
   * G1 (#70): legt einen leeren Eintrag an (Notizblock ohne Audio und Quelle),
   * waehlt ihn, oeffnet ihn in der Mitte und stellt den Titel zum Umbenennen
   * bereit. `folderId`: Projekt (`null` = ohne); `view`: die Zeile der Spalte, die
   * danach gewaehlt ist, damit der Eintrag in der Liste steht.
   */
  const createEmpty = async (folderId: string | null, view: string) => {
    const result = await commands.meetingsCreateEmpty(
      t("meetings.empty.defaultTitle"),
      folderId,
    );
    if (result.status === "error") {
      toast.error(t("meetings.empty.createFailed"));
      return;
    }
    projects.select(view);
    notifyMeetingsChanged();
    select(result.data);
    setCenterTab("transcript");
    setRightTab("notes");
    setAutoRenameId(result.data.id);
    layout.drawer.setOpen(false);
  };

  /** Die drei Wege, einen leeren Eintrag zu fuellen (Startflaeche in der Mitte). */
  const fillEntry = {
    record: () => {
      if (layout.mode !== "narrow" && layout.right.collapsed) {
        layout.right.setCollapsed(false);
      }
      requestStartDialog();
    },
    importFile: () => void importer.pick(),
    link: () => openYoutubeLinkDialog(),
    recordDisabled: recording.active,
    importDisabled: importer.busy,
  };
  const target = useSelectedProject();
  const targetName = target.projectId
    ? projects.folders.find((f) => f.id === target.projectId)?.name
    : undefined;

  /** Chat ueber viele Besprechungen oeffnen (ohne Brief-Auftrag). */
  const openGlobal = useCallback(
    (filter: ScopeFilter | null) => {
      setBriefRun(null);
      setGlobalFilter(filter);
      if (filter) setRightTab("chat");
    },
    [setRightTab],
  );

  /**
   * Brief zu einem Termin: mit gespeichertem Verlauf diesen oeffnen (zweiter
   * Klick), sonst das Recipe mit den Teilnehmenden ausfuehren.
   */
  const openBrief = useCallback(
    (info: BriefInfo) => {
      const nonce = Date.now();
      setGlobalFilter(info.filter);
      setRightTab("chat");
      setBriefRun(
        info.thread_id
          ? { recipe: null, thread: { id: info.thread_id, nonce } }
          : {
              thread: null,
              recipe: {
                id: info.recipe_id,
                nonce,
                values: { [info.recipe_var]: info.recipe_value },
                display: t("meetings.people.brief.question", {
                  names: info.recipe_value,
                }),
              },
            },
      );
    },
    [t, setRightTab],
  );

  // "Vorbereiten" im Hinweisfenster: das Backend merkt den Termin und meldet
  // ihn; beim Start der Seite liegt er womoeglich schon bereit.
  useEffect(() => {
    const openPending = async () => {
      let key: string | null = null;
      try {
        key = await commands.peopleBriefPending();
      } catch {
        return;
      }
      if (!key) return;
      const result = await commands.peopleBriefInfo(key);
      if (result.status !== "ok") {
        toast.error(t("meetings.people.brief.error"));
        return;
      }
      if (result.data.shared_meetings === 0) {
        toast.info(t("meetings.people.brief.none"));
        return;
      }
      openBrief(result.data);
    };
    void openPending();
    const un = events.briefRequestEvent.listen(() => void openPending());
    return () => {
      void un.then((f) => f());
    };
  }, [openBrief, t]);

  const openCitation = useCallback(
    async (citation: Citation) => {
      const request = { citation, nonce: Date.now() };
      if (selected?.id === citation.meeting_id) {
        setJump(request);
        return;
      }
      const meeting = await findMeeting(citation.meeting_id);
      if (!meeting) {
        toast.error(t("meetings.chat.citation.deleted"));
        return;
      }
      select(meeting);
      setJump(request);
    },
    [selected, t, select],
  );

  // C5 (#68): Sprung aus dem Laufprotokoll der Automationen zu einem Segment, das ein
  // Agent-Schritt als Quelle nennt (Herkunft).
  useOpenSegment((request) => {
    void openCitation({
      n: 0,
      meeting_id: request.meetingId,
      meeting_title: "",
      started_at: null,
      source: "transcript",
      epoch: 0,
      segment_index: request.segmentIndex,
      start_ms: null,
      ref_key: null,
      quote: "",
    });
  });

  // ---- G3 (#70, U9): Projekt-Protokoll ----------------------------------------

  /** Fehlertext zu einem Code des Projekt-Protokolls (eigene Texte, sonst die des Protokolls). */
  const pmErrorText = useCallback(
    async (raw: string): Promise<string> => {
      const code = projectMinutesErrorCode(raw);
      const detail = projectMinutesErrorDetail(raw);
      const own = `meetings.projectMinutes.errors.${code}`;
      if (i18n.exists(own)) {
        // Die abgewiesene Aufnahme beim Namen nennen, wenn sie sich finden laesst.
        if (
          (code === "no_transcript" || code === "meeting_not_finished") &&
          detail
        ) {
          const meeting = await findMeeting(detail);
          if (meeting) return t(`${own}_named`, { title: meeting.title });
        }
        return t(own);
      }
      return t(`meetings.minutes.errors.${code}`, {
        error: detail,
        defaultValue: raw,
      });
    },
    [i18n, t],
  );

  const closeRun = useCallback((folderId: string) => {
    setOpenPm((cur) =>
      cur?.kind === "run" && cur.folderId === folderId ? null : cur,
    );
    setPmRunError((cur) => (cur?.folderId === folderId ? null : cur));
  }, []);

  const showRunFailure = useCallback(
    async (folderId: string, raw: string) => {
      const code = projectMinutesErrorCode(raw);
      // Ein zweiter Start: der erste Lauf laeuft weiter und bleibt zu sehen.
      if (code === "minutes_busy") return;
      if (code === "minutes_cancelled") {
        closeRun(folderId);
        toast.info(t("meetings.projectMinutes.run.cancelled"));
        return;
      }
      const text = await pmErrorText(raw);
      setPmRunError({ folderId, text });
    },
    [closeRun, pmErrorText, t],
  );

  /** Ende eines Laufs (auch eines, den ein frueher geoeffneter Reiter gestartet hat). */
  useEffect(() => {
    const un = events.projectMinutesEvent.listen((event) => {
      const payload = event.payload;
      if (payload.kind === "done") {
        setOpenPm((cur) =>
          cur?.kind === "run" && cur.folderId === payload.folder_id
            ? { kind: "doc", id: payload.minutes_id }
            : cur,
        );
      } else {
        void showRunFailure(
          payload.folder_id,
          payload.detail ? `${payload.code}: ${payload.detail}` : payload.code,
        );
      }
    });
    return () => {
      void un.then((f) => f());
    };
  }, [showRunFailure]);

  const startProjectMinutes = useCallback(
    async (request: {
      folderId: string;
      meetingIds: string[];
      templateId: string;
      kind: "minutes" | "summary";
    }) => {
      setPmRunError(null);
      select(null);
      setOpenPm({ kind: "run", folderId: request.folderId });
      layout.drawer.setOpen(false);
      const result = await commands.projectMinutesGenerate(
        request.folderId,
        request.meetingIds,
        request.templateId,
        request.kind,
      );
      if (result.status === "ok") {
        notifyProjectMinutesChanged();
        setOpenPm((cur) =>
          cur?.kind === "run" && cur.folderId === request.folderId
            ? { kind: "doc", id: result.data.id }
            : cur,
        );
        return;
      }
      await showRunFailure(request.folderId, String(result.error));
    },
    [select, showRunFailure, layout.drawer],
  );

  const openPmDoc = useCallback(
    (id: string) => {
      select(null);
      setOpenPm({ kind: "doc", id });
      layout.drawer.setOpen(false);
    },
    [select, layout.drawer],
  );

  const openPmRun = useCallback(
    (folderId: string) => {
      select(null);
      setOpenPm({ kind: "run", folderId });
      layout.drawer.setOpen(false);
    },
    [select, layout.drawer],
  );

  /** Ein Beleg: die Aufnahme oeffnen und zur Stelle springen (Sprungmechanik der Einzelansicht). */
  const openPmSource = useCallback(
    (source: EntrySource, doc: ProjectMinutes) => {
      const title =
        doc.recordings.find((r) => r.index === source.recording)?.title ?? "";
      void openCitation(sourceToCitation(source, title));
    },
    [openCitation],
  );

  const openPmRecording = useCallback(
    async (recording: SourceRecording) => {
      const meeting = await findMeeting(recording.meeting_id);
      if (!meeting) {
        toast.error(t("meetings.projectMinutes.view.sourceDeleted"));
        return;
      }
      select(meeting);
    },
    [select, t],
  );

  const pmContent = openPm ? (
    openPm.kind === "doc" ? (
      <ProjectMinutesView
        key={openPm.id}
        id={openPm.id}
        onOpenSource={openPmSource}
        onOpenRecording={(recording) => void openPmRecording(recording)}
        onDeleted={() => setOpenPm(null)}
      />
    ) : (
      <ProjectMinutesRun
        key={openPm.folderId}
        progress={progressMap[projectMinutesJobKey(openPm.folderId)]}
        error={
          pmRunError?.folderId === openPm.folderId ? pmRunError.text : null
        }
        onClose={() => closeRun(openPm.folderId)}
      />
    )
  ) : null;

  const chatOpen = rightTab === "chat" && globalFilter === null;
  const toggleChat = useCallback(() => {
    if (chatOpen) {
      setRightTab(notesTabRef.current);
      return;
    }
    // Ein globaler Chat weicht dem Chat dieser Besprechung.
    openGlobal(null);
    setRightTab("chat");
  }, [chatOpen, openGlobal, setRightTab]);

  const hint = (text: string) => (
    <p className="p-3 text-sm text-text/60">{text}</p>
  );

  // Laeuft gerade die Aufnahme DIESER Besprechung?
  const live =
    selected !== null &&
    recording.active &&
    recording.meetingId === selected.id;

  const chatBody = globalFilter ? (
    <ChatPanel
      fill
      scope={{ kind: "global", filter: globalFilter }}
      mode="global"
      onClose={() => {
        openGlobal(null);
        setRightTab(notesTabRef.current);
      }}
      onJump={(c) => void openCitation(c)}
      onScopeChange={setGlobalFilter}
      autoRecipe={briefRun?.recipe ?? null}
      openThread={briefRun?.thread ?? null}
    />
  ) : selected ? (
    <ChatPanel
      fill
      key={selected.id}
      scope={{ kind: "meeting", meeting_id: selected.id }}
      mode={live ? "live" : "meeting"}
      onClose={() => setRightTab(notesTabRef.current)}
      onJump={(c) => void openCitation(c)}
    />
  ) : (
    hint(t("meetings.layout.emptyChat"))
  );

  return (
    <PageShell
      fill
      title={t("workspace.recordings")}
      description={
        layout.mode === "narrow" ? undefined : t("workspace.meetingsHint")
      }
      help="aufnahmen"
      actions={
        layout.mode === "narrow" ? (
          <button
            type="button"
            onClick={() => layout.drawer.setOpen(true)}
            title={t("meetings.projects.open")}
            aria-label={t("meetings.projects.open")}
            data-testid="sessions-open"
            className="p-1.5 rounded-md text-text/60 hover:text-text hover:bg-mid-gray/20 transition-colors cursor-pointer"
          >
            <PanelLeftOpen width={20} height={20} aria-hidden="true" />
          </button>
        ) : undefined
      }
    >
      <RecWorkspace
        layout={layout}
        projectsActionsRef={setProjectsActionsEl}
        projectsRail={(open) => (
          <ProjectsRail
            folders={projects.folders}
            counts={projects.counts}
            selection={projects.selection}
            onPick={(id) => {
              projects.select(id);
              open();
            }}
          />
        )}
        projectsBody={
          <MeetingList
            projects={projects}
            drag={drag}
            actionsEl={projectsActionsEl}
            onSelect={(meeting) => {
              select(meeting);
              layout.drawer.setOpen(false);
            }}
            selected={selected}
            onDeleted={(id) => {
              if (selected?.id === id) select(null);
            }}
            onAsk={openGlobal}
            onPrepare={openBrief}
            liveId={recording.active ? recording.meetingId : null}
            personFilter={personFilter}
            onPersonFilterChange={setPersonFilter}
            onNewMeeting={(folderId, view) => void createEmpty(folderId, view)}
            onStartProjectMinutes={(request) =>
              void startProjectMinutes(request)
            }
            onOpenProjectMinutes={openPmDoc}
            onOpenProjectRun={openPmRun}
            activeProjectMinutes={openPm}
          />
        }
        detailActive={selected !== null}
        slotRefs={{
          content: setContentEl,
          controls: setControlsEl,
          notes: setNotesEl,
        }}
        idleContent={pmContent ?? hint(t("meetings.layout.emptyContent"))}
        controls={
          <RecorderCard
            target={emptyTarget}
            onStarted={startedHere}
            importApi={{
              busy: importer.busy,
              pick: () => void importer.pick(),
            }}
          />
        }
        rightTab={rightTab}
        onRightTab={setRightTab}
        idleNotes={hint(t("meetings.layout.emptyNotes"))}
        chatBody={chatBody}
        dropOverlay={
          dropOver ? (
            <div
              data-testid="drop-overlay"
              className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-lg border-2 border-dashed border-logo-primary bg-background/85 p-4 text-center text-sm font-medium text-logo-primary"
            >
              {emptyTarget
                ? t("meetings.empty.dropIntoEntry", {
                    title: emptyTarget.title,
                  })
                : targetName
                  ? t("meetings.import.dropInto", { name: targetName })
                  : t("meetings.import.dropIntoNone")}
            </div>
          ) : null
        }
      />
      {importer.dialog}
      <DragGhost drag={drag.drag} />
      <YoutubeLinkHost onCreated={select} target={emptyTarget} />
      {selected && (
        <MeetingDetail
          key={selected.id}
          meeting={selected}
          slots={{
            content: contentEl,
            controls: controlsEl,
            notes: notesEl,
          }}
          centerTab={centerTab}
          onCenterTab={setCenterTab}
          notesTab={notesTab}
          onLowerTab={setRightTab}
          chatOpen={chatOpen}
          onChatToggle={toggleChat}
          onMeetingChange={setSelected}
          onDeleted={() => select(null)}
          jumpRequest={jump}
          onPersonFilter={setPersonFilter}
          onPersonAsk={(person) =>
            openGlobal({ ...EMPTY_SCOPE, person_id: person.id })
          }
          live={live}
          compact={layout.mode === "narrow"}
          fill={fillEntry}
          autoRename={autoRenameId === selected.id}
          onAutoRenameStarted={() => setAutoRenameId(null)}
        />
      )}
    </PageShell>
  );
};
