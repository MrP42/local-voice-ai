import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ChevronRight,
  MessageSquare,
  PanelLeftOpen,
  Sparkles,
} from "lucide-react";
import { toast } from "sonner";
import { PageShell } from "../../ui/PageShell";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { usePersistentState } from "../../../hooks/usePersistentState";
import { useRecordingActive } from "../../../hooks/useRecordingActive";
import {
  commands,
  events,
  type BriefInfo,
  type Citation,
  type Meeting,
  type RecipeItem,
  type ScopeFilter,
} from "@/bindings";
import { RecorderCard } from "./RecorderCard";
import { LiveTranscript } from "./LiveTranscript";
import { LiveNotesPad } from "./notes/LiveNotesPad";
import { MeetingList } from "./MeetingList";
import { MeetingDetail } from "./MeetingDetail";
import { RecWorkspace, isRightTab, type RightTab } from "./RecWorkspace";
import { useRecLayout } from "./useRecLayout";
import { ChatPanel } from "./chat/ChatPanel";
import { recipeTitleText } from "./chat/RecipeMenu";
import { EMPTY_SCOPE } from "./chat/ScopeChips";
import type { PersonRef } from "./people/PersonPopover";

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

/**
 * M4-P4e: eingeklappte Zeile "Frage zur laufenden Besprechung" unter dem
 * Notizblock. Erscheint mit einer laufenden Aufnahme; die live-tauglichen
 * Recipes ("Was habe ich verpasst?") fragen mit einem Klick.
 */
const LiveChatRow: React.FC<{ onJump: (citation: Citation) => void }> = ({
  onJump,
}) => {
  const { t } = useTranslation();
  const [meetingId, setMeetingId] = useState<string | null>(null);
  const [recording, setRecording] = useState(false);
  const [open, setOpen] = useState(false);
  const [recipes, setRecipes] = useState<RecipeItem[]>([]);
  const [auto, setAuto] = useState<{ id: string; nonce: number } | null>(null);

  useEffect(() => {
    let cancelled = false;
    void commands.meetingsRecordingPosition().then((result) => {
      if (!cancelled && result.status === "ok" && result.data) {
        setMeetingId((prev) => prev ?? result.data!.meeting_id);
        setRecording(true);
      }
    });
    const un = events.meetingEvent.listen((e) => {
      const payload = e.payload;
      if (payload.kind !== "state") return;
      setRecording(
        payload.status === "recording" || payload.status === "paused",
      );
      if (payload.status === "recording") setMeetingId(payload.meeting_id);
    });
    return () => {
      cancelled = true;
      un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    if (!meetingId) return;
    void commands.chatRecipesList().then((result) => {
      if (result.status !== "ok") return;
      setRecipes(
        (result.data ?? []).filter(
          (r) => r.spec.live_ok && r.spec.scope !== "global",
        ),
      );
    });
  }, [meetingId]);

  if (!meetingId || (!recording && !open)) return null;

  return (
    <SettingsGroup>
      <div className="space-y-2 px-4 py-2" data-testid="live-chat-row">
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={() => setOpen((o) => !o)}
            aria-expanded={open}
            aria-label={
              open
                ? t("meetings.chat.live.collapse")
                : t("meetings.chat.live.expand")
            }
            className="flex items-center gap-1.5 text-sm font-medium text-text/80 hover:text-text cursor-pointer"
          >
            <ChevronRight
              width={14}
              height={14}
              aria-hidden="true"
              className={`transition-transform ${open ? "rotate-90" : ""}`}
            />
            <MessageSquare width={14} height={14} aria-hidden="true" />
            {t("meetings.chat.live.row")}
          </button>
          {!open &&
            recipes.map((recipe) => (
              <button
                key={recipe.id}
                type="button"
                onClick={() => {
                  setOpen(true);
                  setAuto({ id: recipe.id, nonce: Date.now() });
                }}
                className="inline-flex items-center gap-1 rounded-full border border-mid-gray/40 px-2 py-0.5 text-xs text-text/70 hover:bg-mid-gray/15 hover:text-text cursor-pointer"
              >
                <Sparkles width={10} height={10} aria-hidden="true" />
                {recipeTitleText(recipe)}
              </button>
            ))}
        </div>
        {open && (
          <ChatPanel
            scope={{ kind: "meeting", meeting_id: meetingId }}
            mode={recording ? "live" : "meeting"}
            onJump={onJump}
            autoRecipe={auto}
          />
        )}
      </div>
    </SettingsGroup>
  );
};

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

  // Die Bereiche, in die die Detailansicht ihre Teile legt.
  const [contentEl, setContentEl] = useState<HTMLDivElement | null>(null);
  const [controlsEl, setControlsEl] = useState<HTMLDivElement | null>(null);
  const [transcriptEl, setTranscriptEl] = useState<HTMLDivElement | null>(null);

  // Gemerkte Besprechung beim Start laden; gibt es sie nicht mehr, vergessen.
  useEffect(() => {
    if (!selectedId || selected) return;
    let cancelled = false;
    void findMeeting(selectedId).then((meeting) => {
      if (cancelled) return;
      if (meeting) setSelected(meeting);
      else setSelectedId("");
    });
    return () => {
      cancelled = true;
    };
    // Nur beim Einhaengen: spaetere Auswahlen setzen `selected` selbst.
  }, []);

  // Eine neue Aufnahme zeigt ihren Notizblock in der Arbeitsflaeche.
  useEffect(() => {
    if (recording.meetingId) select(null);
  }, [recording.meetingId, select]);

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
      mode="meeting"
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
        projectsBody={
          <MeetingList
            onSelect={(meeting) => {
              select(meeting);
              layout.drawer.setOpen(false);
            }}
            selected={selected}
            onDeleted={(id) => {
              if (selected?.id === id) select(null);
            }}
            onAsk={openGlobal}
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
        idleContent={
          <div className="space-y-3 p-3">
            <LiveNotesPad fallback={hint(t("meetings.layout.emptyContent"))} />
            <LiveChatRow onJump={(c) => void openCitation(c)} />
          </div>
        }
        controls={<RecorderCard onPrepare={openBrief} />}
        rightTab={rightTab}
        onRightTab={setRightTab}
        idleTranscript={
          <LiveTranscript
            fallback={hint(t("meetings.layout.emptyTranscript"))}
          />
        }
        chatBody={chatBody}
      />
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
          jumpRequest={jump}
          onPersonFilter={setPersonFilter}
          onPersonAsk={(person) =>
            openGlobal({ ...EMPTY_SCOPE, person_id: person.id })
          }
        />
      )}
    </PageShell>
  );
};
