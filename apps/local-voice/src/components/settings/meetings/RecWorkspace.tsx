import React, { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  PanelLeftClose,
  PanelLeftOpen,
  PanelRightClose,
  PanelRightOpen,
  X,
} from "lucide-react";
import { ResizeHandle } from "../../ui/ResizeHandle";
import { TabList } from "../../ui/TabList";
import type { RecLayout } from "./useRecLayout";
import { useStableSlot } from "./useStableSlot";

/** Reiter der unteren rechten Flaeche. */
export type RightTab = "transcript" | "chat";
export const isRightTab = (value: string): value is RightTab =>
  value === "transcript" || value === "chat";

/** Die Stellen, an denen die Detailansicht ihre Teile einhaengt. */
export interface RecSlotRefs {
  /** Arbeitsflaeche: Kopf, Reiter Notizen / KI-Notizen / Protokoll. */
  content: (el: HTMLDivElement | null) => void;
  /** Bedienung: Aktionen, Fortschritt, Wiedergabe. */
  controls: (el: HTMLDivElement | null) => void;
  /** Transkript (Reiter rechts unten). */
  transcript: (el: HTMLDivElement | null) => void;
}

const ICON_BUTTON =
  "p-1 rounded-md text-text/60 hover:text-text hover:bg-mid-gray/20 transition-colors cursor-pointer disabled:opacity-30 disabled:cursor-default";

/**
 * Horizontaler Griff zwischen Arbeitsflaeche und rechtem Bereich im
 * gestapelten Layout. Der Wert ist der Anteil der Arbeitsflaeche in Prozent;
 * gezogen wird absolut zur Hoehe des Containers. Pfeil hoch/runter = 5 %,
 * Pos1/Ende = Grenzen, Doppelklick = halbe-halbe.
 */
const SplitHandle: React.FC<{
  container: React.RefObject<HTMLDivElement | null>;
  value: number;
  min: number;
  max: number;
  defaultValue: number;
  label: string;
  onChange: (value: number) => void;
}> = ({ container, value, min, max, defaultValue, label, onChange }) => {
  const dragging = useRef(false);
  const clamp = (v: number) => Math.min(Math.max(v, min), max);

  const move = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging.current) return;
    const rect = container.current?.getBoundingClientRect();
    if (!rect || rect.height <= 0) return;
    onChange(clamp(((event.clientY - rect.top) / rect.height) * 100));
  };

  return (
    <div
      role="separator"
      aria-orientation="horizontal"
      aria-valuenow={Math.round(value)}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-label={label}
      title={label}
      tabIndex={0}
      data-testid="resize-split"
      className="rec-hsplit"
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.currentTarget.setPointerCapture(event.pointerId);
        dragging.current = true;
        event.preventDefault();
      }}
      onPointerMove={move}
      onPointerUp={() => {
        dragging.current = false;
      }}
      onPointerCancel={() => {
        dragging.current = false;
      }}
      onKeyDown={(event) => {
        let next: number | null = null;
        if (event.key === "ArrowUp") next = value - 5;
        else if (event.key === "ArrowDown") next = value + 5;
        else if (event.key === "Home") next = min;
        else if (event.key === "End") next = max;
        if (next === null) return;
        event.preventDefault();
        onChange(clamp(next));
      }}
      onDoubleClick={() => onChange(defaultValue)}
    />
  );
};

interface RecWorkspaceProps {
  layout: RecLayout;
  /** Projekte und ihre Besprechungen (MeetingList); scrollt selbst. */
  projectsBody: React.ReactNode;
  /** Platz im Kopf der Projekte-Spalte fuer Symbolknoepfe (Portal-Ziel). */
  projectsActionsRef: (el: HTMLDivElement | null) => void;
  /** Eingeklappte Leiste: Projekte als Kuerzel; `open` oeffnet Spalte bzw. Schublade. */
  projectsRail: (open: () => void) => React.ReactNode;
  /** Gewaehlte Besprechung: die Detailansicht haengt ihre Teile in die Slots. */
  detailActive: boolean;
  slotRefs: RecSlotRefs;
  /** Arbeitsflaeche ohne gewaehlte Besprechung (Hinweis). */
  idleContent: React.ReactNode;
  /** Bedienung oben rechts (Aufnahmekarte). */
  controls: React.ReactNode;
  rightTab: RightTab;
  onRightTab: (tab: RightTab) => void;
  /** Transkript ohne gewaehlte Besprechung (Hinweis). */
  idleTranscript: React.ReactNode;
  /** Fragen-Reiter (Chat). */
  chatBody: React.ReactNode;
  /** Ablagefeld ueber der Arbeitsflaeche, solange eine Datei darueber schwebt. */
  dropOverlay?: React.ReactNode;
}

/**
 * Geruest der Aufnahmen-Seite, Variante B: Projekte | Arbeitsflaeche |
 * Bedienung und Transkript. Jeder Bereich scrollt fuer sich, die Seite nie.
 *
 * - breit (>= 1000 px): drei Bereiche, Ziehgriffe, einklappbar.
 * - mittel (620 bis 999 px): Projekte als Leiste mit Schublade.
 * - schmal (< 620 px): oben die Aufnahmezeile, darunter geteilt die
 *   Arbeitsflaeche (Notizen) und das Transkript bzw. die Fragen, dazwischen
 *   ein waagerechter Griff; Projekte in der Schublade.
 *
 * Die drei Hauptteile (Arbeitsflaeche, Aufnahmezeile samt Bedienung,
 * Transkript/Fragen) sitzen in festen Elementen, die je nach Breite nur an
 * einen anderen Platz gehaengt werden (`useStableSlot`): Ein Fensterwechsel
 * waehrend einer Aufnahme baut weder den Notizblock noch das Live-Transkript
 * oder die Aufnahmekarte neu.
 */
export const RecWorkspace: React.FC<RecWorkspaceProps> = ({
  layout,
  projectsBody,
  projectsActionsRef,
  projectsRail,
  detailActive,
  slotRefs,
  idleContent,
  controls,
  rightTab,
  onRightTab,
  idleTranscript,
  chatBody,
  dropOverlay,
}) => {
  const { t } = useTranslation();
  const { mode, sessions, right, split, sessionsColumn } = layout;
  const { open: drawerWanted, setOpen: setDrawerOpen } = layout.drawer;
  const narrow = mode === "narrow";
  const drawerOpen = drawerWanted && !sessionsColumn;
  const showColumn = sessionsColumn || drawerOpen;
  const containerRef = layout.ref;
  const splitRef = useRef<HTMLDivElement | null>(null);
  const contentSlot = useStableSlot("flex min-h-0 min-w-0 flex-1 flex-col");
  const controlsSlot = useStableSlot(
    "max-h-full min-h-0 space-y-3 overflow-y-auto",
    "rec-controls-area",
  );
  const lowerSlot = useStableSlot("flex min-h-0 min-w-0 flex-1 flex-col");

  // Nach einer Groessenaenderung kann die Schublade ueberfluessig werden.
  useEffect(() => {
    if (sessionsColumn) setDrawerOpen(false);
  }, [sessionsColumn, setDrawerOpen]);

  useEffect(() => {
    if (!drawerOpen) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setDrawerOpen(false);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [drawerOpen, setDrawerOpen]);

  const projectsTitle = t("meetings.projects.title");

  // Beide Reiter der unteren Flaeche bleiben eingehaengt und werden nur
  // verborgen: ein Live-Transkript sammelt Saetze aus Ereignissen, ein Chat
  // haelt eine laufende Antwort -- beides ginge beim Reiterwechsel verloren.
  // Den Chat gibt es erst nach dem ersten Oeffnen (er laedt beim Einhaengen).
  const [chatSeen, setChatSeen] = useState(rightTab === "chat");
  useEffect(() => {
    if (rightTab === "chat") setChatSeen(true);
  }, [rightTab]);

  const sessionsColumnEl = (
    <div
      data-testid={showColumn ? "rec-sessions" : undefined}
      role="region"
      aria-label={projectsTitle}
      className={
        drawerOpen
          ? "rec-drawer absolute inset-y-0 start-0 z-30 flex w-[min(22rem,100%)] flex-col gap-1 rounded-lg border border-mid-gray/30 bg-background p-2 shadow-xl"
          : showColumn
            ? "rec-sessions flex min-h-0 shrink-0 flex-col gap-1"
            : "hidden"
      }
      style={sessionsColumn ? { width: `${sessions.width}px` } : undefined}
    >
      <div className="flex items-center justify-between px-1">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-text/60">
          {projectsTitle}
        </h2>
        <div className="flex items-center gap-1">
          <div
            ref={projectsActionsRef}
            className="flex items-center gap-1"
            data-testid="projects-actions"
          />
          {drawerOpen ? (
            <button
              type="button"
              className={ICON_BUTTON}
              title={t("meetings.projects.close")}
              aria-label={t("meetings.projects.close")}
              onClick={() => setDrawerOpen(false)}
            >
              <X width={16} height={16} aria-hidden="true" />
            </button>
          ) : (
            <button
              type="button"
              className={ICON_BUTTON}
              title={t("meetings.projects.collapse")}
              aria-label={t("meetings.projects.collapse")}
              data-testid="sessions-collapse"
              onClick={() => sessions.setCollapsed(true)}
            >
              <PanelLeftClose width={16} height={16} aria-hidden="true" />
            </button>
          )}
        </div>
      </div>
      {/* Die Liste scrollt in sich (Suche oben, "Als Naechstes" unten stehen
          fest): hier darf nichts scrollen. */}
      <div
        data-testid="rec-sessions-scroll"
        className="min-h-0 flex-1 overflow-hidden"
      >
        {projectsBody}
      </div>
    </div>
  );

  // Eingeklappt (oder zu schmal fuer die Spalte): eine Leiste mit dem Knopf,
  // der die Spalte bzw. die Schublade oeffnet.
  const openProjects = () =>
    mode === "wide" ? sessions.setCollapsed(false) : setDrawerOpen(true);
  const sessionsRail =
    !narrow && !showColumn ? (
      <div
        data-testid="rec-sessions"
        role="region"
        aria-label={projectsTitle}
        className="flex min-h-0 shrink-0 flex-col items-center pt-1"
        style={{ width: `${sessions.width}px` }}
      >
        <button
          type="button"
          className={ICON_BUTTON}
          title={t("meetings.projects.expand")}
          aria-label={t("meetings.projects.expand")}
          data-testid="sessions-expand"
          onClick={openProjects}
        >
          <PanelLeftOpen width={16} height={16} aria-hidden="true" />
        </button>
        {projectsRail(openProjects)}
      </div>
    ) : null;

  // --- Die drei Hauptteile (leben in festen Elementen, siehe oben) ---------

  const contentCard = (
    <div
      data-testid="rec-content"
      data-rec-dropzone=""
      role="region"
      aria-label={t("meetings.layout.content")}
      className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-lg border border-mid-gray/20 bg-background"
    >
      <div
        data-testid="rec-content-scroll"
        className="min-h-0 flex-1 overflow-y-auto"
      >
        {detailActive ? (
          <div ref={slotRefs.content} className="space-y-3 px-4 py-3" />
        ) : (
          idleContent
        )}
      </div>
      {dropOverlay}
    </div>
  );

  const controlsPiece = (
    <>
      {controls}
      {detailActive && <div ref={slotRefs.controls} className="space-y-3" />}
    </>
  );

  const transcriptTabs = [
    { id: "transcript" as const, label: t("meetings.detail.transcriptTab") },
    { id: "chat" as const, label: t("meetings.chat.ask") },
  ];

  const lowerCard = (
    <div
      data-testid="rec-lower"
      className="flex min-h-[8rem] min-w-0 flex-1 flex-col overflow-hidden rounded-lg border border-mid-gray/20 bg-background"
    >
      <TabList
        compact
        tabs={transcriptTabs}
        value={rightTab}
        onChange={onRightTab}
        ariaLabel={t("meetings.layout.lowerTabs")}
        className="shrink-0 border-b border-mid-gray/20 px-3"
      />
      <div
        role="tabpanel"
        hidden={rightTab !== "transcript"}
        className={
          rightTab === "transcript" ? "flex min-h-0 flex-1 flex-col" : "hidden"
        }
        data-testid="rec-transcript"
      >
        {detailActive ? (
          <div
            ref={slotRefs.transcript}
            className="flex min-h-0 flex-1 flex-col gap-2 px-3 py-2"
          />
        ) : (
          idleTranscript
        )}
      </div>
      {(chatSeen || rightTab === "chat") && (
        <div
          role="tabpanel"
          hidden={rightTab !== "chat"}
          className={
            rightTab === "chat" ? "flex min-h-0 flex-1 flex-col" : "hidden"
          }
          data-testid="rec-chat"
        >
          {chatBody}
        </div>
      )}
    </div>
  );

  // --- Plaetze der drei Hauptteile je nach Breite ---------------------------

  const contentHost = (
    <div
      ref={contentSlot.host}
      className="flex min-h-0 min-w-0 flex-col"
      style={narrow ? { flex: `${split.value} 1 0` } : { flex: "1 1 0" }}
    />
  );

  const rightColumn = (
    <div
      data-testid="rec-controls"
      role="region"
      aria-label={t("meetings.layout.controls")}
      className="flex min-h-0 min-w-0 flex-col gap-2"
      style={{ width: `${right.width}px`, flexShrink: 0 }}
    >
      <div className="flex items-center justify-between px-1">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-text/60">
          {t("meetings.layout.controls")}
        </h2>
        <button
          type="button"
          className={ICON_BUTTON}
          title={t("meetings.layout.rightCollapse")}
          aria-label={t("meetings.layout.rightCollapse")}
          data-testid="right-collapse"
          // Waehrend einer Aufnahme sitzt dort der Stopp-Knopf.
          disabled={layout.recording}
          onClick={() => right.setCollapsed(true)}
        >
          <PanelRightClose width={16} height={16} aria-hidden="true" />
        </button>
      </div>
      <div
        className="flex min-h-0 flex-col"
        style={{ flex: "0 1 auto", maxHeight: "55%" }}
      >
        <div ref={controlsSlot.host} className="flex min-h-0 flex-col" />
      </div>
      <div
        ref={lowerSlot.host}
        className="flex min-h-[8rem] min-w-0 flex-1 flex-col"
      />
    </div>
  );

  const rightRail = (
    <div
      data-testid="rec-controls"
      role="region"
      aria-label={t("meetings.layout.controls")}
      className="flex shrink-0 flex-col items-center pt-1"
      style={{ width: `${right.width}px` }}
    >
      <button
        type="button"
        className={ICON_BUTTON}
        title={t("meetings.layout.rightExpand")}
        aria-label={t("meetings.layout.rightExpand")}
        data-testid="right-expand"
        onClick={() => right.setCollapsed(false)}
      >
        <PanelRightOpen width={16} height={16} aria-hidden="true" />
      </button>
    </div>
  );

  return (
    <>
      <div
        ref={containerRef}
        data-mode={mode}
        className={`rec-workspace relative flex h-full min-h-0 w-full items-stretch ${
          narrow ? "flex-col gap-2" : "gap-4"
        }`}
      >
        {sessionsRail}
        {sessionsColumnEl}
        {drawerOpen && (
          <div
            className="absolute inset-0 z-20 rounded-lg bg-black/30"
            onClick={() => setDrawerOpen(false)}
            aria-hidden="true"
          />
        )}
        {sessionsColumn && (
          <ResizeHandle
            testId="resize-sessions"
            label={t("meetings.projects.resize")}
            direction={1}
            value={sessions.width}
            min={sessions.min}
            max={sessions.max}
            defaultValue={sessions.def}
            onChange={sessions.set}
          />
        )}
        {narrow ? (
          <>
            {/* Aufnahmezeile (Start/Import oder laufende Aufnahme) samt
                Wiedergabe und Fortschritt: steht fest ueber der Teilung. */}
            <div
              data-testid="rec-strip"
              role="region"
              aria-label={t("meetings.layout.controls")}
              className="flex min-h-0 shrink-0 flex-col"
              style={{ maxHeight: "40%" }}
            >
              <div ref={controlsSlot.host} className="flex min-h-0 flex-col" />
            </div>
            <div
              ref={splitRef}
              className="flex min-h-0 flex-1 flex-col gap-2"
              data-testid="rec-split"
            >
              {contentHost}
              <SplitHandle
                container={splitRef}
                value={split.value}
                min={split.min}
                max={split.max}
                defaultValue={split.def}
                label={t("meetings.layout.split")}
                onChange={split.set}
              />
              <div
                data-testid="rec-controls"
                role="region"
                aria-label={t("meetings.layout.lowerTabs")}
                className="flex min-h-0 min-w-0 flex-col"
                style={{ flex: `${100 - split.value} 1 0` }}
              >
                <div
                  ref={lowerSlot.host}
                  className="flex min-h-0 flex-1 flex-col"
                />
              </div>
            </div>
          </>
        ) : (
          <>
            {contentHost}
            {!right.collapsed && (
              <ResizeHandle
                testId="resize-right"
                label={t("meetings.layout.resizeRight")}
                direction={-1}
                value={right.width}
                min={right.min}
                max={right.max}
                defaultValue={right.def}
                onChange={right.set}
              />
            )}
            {right.collapsed ? rightRail : rightColumn}
          </>
        )}
      </div>
      {createPortal(contentCard, contentSlot.el)}
      {createPortal(controlsPiece, controlsSlot.el)}
      {createPortal(lowerCard, lowerSlot.el)}
    </>
  );
};
