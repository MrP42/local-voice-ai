import { useLayoutEffect, useRef, useState } from "react";
import { usePersistentState } from "../../../hooks/usePersistentState";

/** Grenzen und Standardbreiten der verstellbaren Spalten (Pixel). */
export const REC_LIMITS = {
  sessions: { min: 180, max: 400, def: 248 },
  right: { min: 300, max: 640, def: 420 },
} as const;

/** Die Arbeitsflaeche behaelt mindestens diese Breite (Prototyp: 380 px). */
export const CONTENT_MIN = 380;
/** Abstand zwischen den Spalten (Tailwind gap-4). */
const GAP = 16;
/** Eingeklappte Leiste: Symbolknopf plus Innenabstand. */
export const RAIL_WIDTH = 40;
/** Ab dieser Seitenbreite stehen alle drei Bereiche nebeneinander. */
export const WIDE_MIN = 1000;
/** Darunter stapeln sich Arbeitsflaeche und Transkript. */
export const NARROW_MAX = 620;
/** Aufteilung im gestapelten Layout: Anteil der Arbeitsflaeche in Prozent. */
export const SPLIT = { min: 25, max: 75, def: 50 } as const;

export type RecMode = "wide" | "medium" | "narrow";

const clamp = (value: number, min: number, max: number) =>
  Math.round(Math.min(Math.max(value, min), Math.max(min, max)));

const isNumber = (value: string) => Number.isFinite(Number(value));
const isFlag = (value: string) => value === "0" || value === "1";

/** Eine persistente Zahl; Unlesbares faellt auf den Standard zurueck. */
function usePersistentNumber(
  key: string,
  def: number,
): [number, (v: number) => void] {
  const [raw, setRaw] = usePersistentState<string>(key, String(def), isNumber);
  return [Number(raw), (value: number) => setRaw(String(Math.round(value)))];
}

/** Ein persistenter Schalter (gespeichert als "0"/"1"). */
function usePersistentFlag(
  key: string,
  def: boolean,
): [boolean, (v: boolean) => void] {
  const [raw, setRaw] = usePersistentState<string>(
    key,
    def ? "1" : "0",
    isFlag,
  );
  return [raw === "1", (value: boolean) => setRaw(value ? "1" : "0")];
}

/**
 * Layout der Aufnahmen-Arbeitsflaeche (Variante B): Projekte links, Arbeitsflaeche
 * in der Mitte, Bedienung und Transkript rechts.
 *
 * Gespeichert wird der Wunsch des Nutzers (Breiten, Klappzustand, Aufteilung);
 * angezeigt wird, was im gerade vorhandenen Fenster Platz hat. So bleibt eine
 * breite Spalte breit, sobald das Fenster wieder waechst.
 *
 * `recording`: Waehrend einer Aufnahme bleibt die rechte Spalte offen, denn
 * dort sitzt der Stopp-Knopf.
 */
export function useRecLayout(recording: boolean) {
  const [sessionsRaw, setSessions] = usePersistentNumber(
    "meetings.ui.sessionsWidth",
    REC_LIMITS.sessions.def,
  );
  const [rightRaw, setRight] = usePersistentNumber(
    "meetings.ui.rightWidth",
    REC_LIMITS.right.def,
  );
  const [sessionsCollapsed, setSessionsCollapsed] = usePersistentFlag(
    "meetings.ui.sessionsCollapsed",
    false,
  );
  const [rightCollapsedPref, setRightCollapsed] = usePersistentFlag(
    "meetings.ui.rightCollapsed",
    false,
  );
  const [split, setSplit] = usePersistentNumber("meetings.ui.split", SPLIT.def);
  // Schublade mit den Projekten, wenn die Spalte keinen Platz hat.
  const [drawerOpen, setDrawerOpen] = useState(false);

  // Breite des Arbeitsbereichs (nicht des Fensters: links steht die Navigation).
  const ref = useRef<HTMLDivElement | null>(null);
  const [available, setAvailable] = useState(Infinity);
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const measure = () => setAvailable(node.clientWidth);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  const mode: RecMode =
    available >= WIDE_MIN
      ? "wide"
      : available >= NARROW_MAX
        ? "medium"
        : "narrow";
  const stackedRows = mode === "narrow";

  const rightCollapsed = rightCollapsedPref && !recording && !stackedRows;
  // Projekte als Spalte nur im breiten Fenster und wenn nicht eingeklappt;
  // sonst Leiste (mittel) bzw. Schublade.
  const sessionsColumn = mode === "wide" && !sessionsCollapsed;

  const room = available - 2 * GAP;
  const sessionsWish = sessionsColumn
    ? clamp(sessionsRaw, REC_LIMITS.sessions.min, REC_LIMITS.sessions.max)
    : RAIL_WIDTH;
  const sessionsFixed = sessionsColumn ? 0 : RAIL_WIDTH;
  // Erst die rechte Spalte gegen den Wunsch der Projekte, dann die Projekte
  // gegen die tatsaechliche rechte Breite (wie im Prototyp: clampW).
  const rightMax = Math.max(
    REC_LIMITS.right.min,
    Math.min(REC_LIMITS.right.max, room - sessionsWish - CONTENT_MIN),
  );
  const rightWidth = rightCollapsed
    ? RAIL_WIDTH
    : clamp(rightRaw, REC_LIMITS.right.min, rightMax);
  const sessionsMax = Math.max(
    REC_LIMITS.sessions.min,
    Math.min(REC_LIMITS.sessions.max, room - rightWidth - CONTENT_MIN),
  );
  const sessionsWidth = sessionsColumn
    ? clamp(sessionsRaw, REC_LIMITS.sessions.min, sessionsMax)
    : sessionsFixed;

  return {
    ref,
    mode,
    recording,
    sessionsColumn,
    sessions: {
      width: sessionsWidth,
      min: REC_LIMITS.sessions.min,
      max: sessionsMax,
      def: REC_LIMITS.sessions.def,
      set: setSessions,
      collapsed: sessionsCollapsed,
      setCollapsed: setSessionsCollapsed,
    },
    right: {
      width: rightWidth,
      min: REC_LIMITS.right.min,
      max: rightMax,
      def: REC_LIMITS.right.def,
      set: setRight,
      collapsed: rightCollapsed,
      setCollapsed: setRightCollapsed,
    },
    split: {
      value: clamp(split, SPLIT.min, SPLIT.max),
      min: SPLIT.min,
      max: SPLIT.max,
      def: SPLIT.def,
      set: (v: number) => setSplit(clamp(v, SPLIT.min, SPLIT.max)),
    },
    drawer: { open: drawerOpen, setOpen: setDrawerOpen },
  };
}

export type RecLayout = ReturnType<typeof useRecLayout>;
