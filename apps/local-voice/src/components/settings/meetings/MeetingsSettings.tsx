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
  type Meeting,
  type ScopeFilter,
} from "@/bindings";
import { RecorderCard } from "./RecorderCard";
import { MeetingList } from "./MeetingList";
import { MeetingDetail } from "./MeetingDetail";
import { RecWorkspace, isRightTab, type RightTab } from "./RecWorkspace";
import { useProjects } from "./projects/useProjects";
import { useMeetingDrag } from "./projects/useMeetingDrag";
import { ProjectsRail } from "./projects/ProjectsRail";
import { DragGhost } from "./projects/DragGhost";
import { useRecLayout } from "./useRecLayout";
import { ChatPanel } from "./chat/ChatPanel";
import { EMPTY_SCOPE } from "./chat/ScopeChips";
import type { PersonRef } from "./people/PersonPopover";
import { useImportDrop, useMeetingImport } from "./useMeetingImport";
import { useSelectedProject } from "./projects/selectedProject";

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

/** Sucht eine Besprechung seitenweise (es gibt keinen Einzelabruf). */
const findMeeting = async (id: string): Promise<Meeting | null> => {
  const PAGE = 200;
  for (let offset = 0; offset < 50 * PAGE; offset += PAGE) {
    const result = await commands.meetingsList(offset, PAGE);
    if (result.status !== "ok") return null;
    const page = result.data ?? [];
    const hit = page.find((m) => m.id === id);
    if (hit) return hit;
    if (page.length < PAGE) return null;
  }
  return null;
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
  const { t } = useTranslation();
  const [selected, setSelected] = useState<Meeting | null>(null);
  // Die gewaehlte Besprechung bleibt ueber Neuladen, Seitenwechsel und
  // Neustart (nur die ID; der Datensatz kommt frisch aus dem Backend).
  const [selectedId, setSelectedId] = usePersistentState<string>(
    "meetings.selected",
    "",
  );
  const select = useCallback(
    (meeting: Meeting | null) => {
      setSelected(meeting);
      setSelectedId(meeting?.id ?? "");
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
  const [rightTab, setRightTab] = usePersistentState<RightTab>(
    "meetings.rightTab",
    "transcript",
    isRightTab,
  );
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
  const [transcriptEl, setTranscriptEl] = useState<HTMLDivElement | null>(null);

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
      setRightTab("transcript");
    },
    [select, setRightTab],
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
      setRightTab("transcript");
    })();
  }, [recording.active, recording.meetingId, select, setRightTab]);

  // Import: Symbol und Ablage auf der Arbeitsflaeche fuehren in EINEN Weg.
  // Eine laufende Aufnahme behaelt den Fokus; der Import laeuft links mit
  // Fortschritt weiter.
  const recordingActiveRef = useRef(recording.active);
  recordingActiveRef.current = recording.active;
  const importer = useMeetingImport(
    useCallback(
      (meeting: Meeting) => {
        if (recordingActiveRef.current) return;
        select(meeting);
        setRightTab("transcript");
      },
      [select, setRightTab],
    ),
  );
  const dropOver = useImportDrop(importer.ask);
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

  const chatOpen = rightTab === "chat" && globalFilter === null;
  const toggleChat = useCallback(() => {
    if (chatOpen) {
      setRightTab("transcript");
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
        setRightTab("transcript");
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
      onClose={() => setRightTab("transcript")}
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
          />
        }
        detailActive={selected !== null}
        slotRefs={{
          content: setContentEl,
          controls: setControlsEl,
          transcript: setTranscriptEl,
        }}
        idleContent={hint(t("meetings.layout.emptyContent"))}
        controls={
          <RecorderCard
            onStarted={startedHere}
            importApi={{
              busy: importer.busy,
              pick: () => void importer.pick(),
            }}
          />
        }
        rightTab={rightTab}
        onRightTab={setRightTab}
        idleTranscript={hint(t("meetings.layout.emptyTranscript"))}
        chatBody={chatBody}
        dropOverlay={
          dropOver ? (
            <div
              data-testid="drop-overlay"
              className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-lg border-2 border-dashed border-logo-primary bg-background/85 p-4 text-center text-sm font-medium text-logo-primary"
            >
              {targetName
                ? t("meetings.import.dropInto", { name: targetName })
                : t("meetings.import.dropIntoNone")}
            </div>
          ) : null
        }
      />
      {importer.dialog}
      <DragGhost drag={drag.drag} />
      {selected && (
        <MeetingDetail
          key={selected.id}
          meeting={selected}
          slots={{
            content: contentEl,
            controls: controlsEl,
            transcript: transcriptEl,
          }}
          onShowTranscript={() => setRightTab("transcript")}
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
        />
      )}
    </PageShell>
  );
};
