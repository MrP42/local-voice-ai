import { useEffect, useRef, useState } from "react";
import { usePersistentState } from "../../../hooks/usePersistentState";

/** Rechts vom Editor: eine gestapelte Spalte oder zwei nebeneinander. */
export type RightLayout = "stacked" | "side";
const isRightLayout = (value: string): value is RightLayout =>
  value === "stacked" || value === "side";

/** Grenzen und Standardbreiten der verstellbaren Spalten (Pixel). */
export const PANE_LIMITS = {
  pages: { min: 160, max: 420, def: 208 },
  right: { min: 260, max: 560, def: 320 },
  files: { min: 200, max: 520, def: 240 },
} as const;

/** Der Editor darf nie schmaler werden als das. */
const EDITOR_MIN = 360;
/** Abstand zwischen den Spalten (Tailwind gap-4). */
const GAP = 16;
/** Bedienung im Nebeneinander-Layout: w-72 = 18 rem (bei 15-px-Basis 270 px).
 *  Bewusst großzügig mit 288 gerechnet, damit auch eine größere Basisschrift
 *  die Editor-Mindestbreite nicht unterschreitet. */
const CONTROLS_SIDE = 288;
/** Aufklapp-Knopf einer zugeklappten Leiste (Symbol plus Innenabstand). */
const COLLAPSED = 32;

const clamp = (value: number, min: number, max: number) =>
  Math.round(Math.min(Math.max(value, min), Math.max(min, max)));

const isNumber = (value: string) => Number.isFinite(Number(value));

/** Eine persistente Breite; unlesbare Werte fallen auf den Standard zurück. */
function usePersistentWidth(
  key: string,
  def: number,
): [number, (v: number) => void] {
  const [raw, setRaw] = usePersistentState<string>(key, String(def), isNumber);
  return [Number(raw), (value: number) => setRaw(String(Math.round(value)))];
}

/**
 * Layout der Vorlesen-Arbeitsfläche: gewählte Anordnung rechts vom Editor,
 * die drei verstellbaren Breiten und die Grenzen, die sich aus der
 * Fensterbreite ergeben (der Editor behält mindestens 360 px).
 *
 * Gespeichert wird der Wunsch des Nutzers; angezeigt wird der Wert, der im
 * gerade vorhandenen Fenster Platz hat. So bleibt eine breite Spalte breit,
 * sobald das Fenster wieder wächst, statt still verkleinert zu bleiben.
 */
export function useWorkspaceLayout(
  pagesCollapsed: boolean,
  filesCollapsed: boolean,
) {
  const [layout, setLayout] = usePersistentState<RightLayout>(
    "tts.rightLayout",
    "stacked",
    isRightLayout,
  );
  const [pagesRaw, setPages] = usePersistentWidth(
    "tts.pagesWidth",
    PANE_LIMITS.pages.def,
  );
  const [rightRaw, setRight] = usePersistentWidth(
    "tts.rightWidth",
    PANE_LIMITS.right.def,
  );
  const [filesRaw, setFiles] = usePersistentWidth(
    "tts.filesWidth",
    PANE_LIMITS.files.def,
  );

  // Breite des Arbeitsbereichs (nicht des Fensters: links steht die Navigation).
  const ref = useRef<HTMLDivElement | null>(null);
  const [available, setAvailable] = useState(Infinity);
  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    const measure = () => setAvailable(node.clientWidth);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  const stacked = layout === "stacked";
  // Platz neben Seitenliste und Editor, ohne die Editor-Mindestbreite
  // anzutasten. Zwischenräume: Seiten|Editor, Editor|rechts und
  // (nebeneinander) Bedienung|Dateien.
  const gaps = stacked ? 2 * GAP : 3 * GAP;
  const fixedRight = stacked ? 0 : CONTROLS_SIDE;
  const room = available - EDITOR_MIN - gaps - fixedRight;

  // Erst die rechte Spalte gegen den Wunsch der Seitenliste, dann die
  // Seitenliste gegen die tatsächliche rechte Breite: die Summe geht auf, und
  // der Wunsch der Seitenliste hat Vorrang, wenn das Fenster zu schmal wird.
  const rightLimit = stacked ? PANE_LIMITS.right : PANE_LIMITS.files;
  const rightWanted = stacked ? rightRaw : filesRaw;
  const rightCollapsed = !stacked && filesCollapsed;
  const pagesWish = pagesCollapsed
    ? COLLAPSED
    : clamp(pagesRaw, PANE_LIMITS.pages.min, PANE_LIMITS.pages.max);
  const rightMax = Math.max(
    rightLimit.min,
    Math.min(rightLimit.max, room - pagesWish),
  );
  const rightWidth = rightCollapsed
    ? COLLAPSED
    : clamp(rightWanted, rightLimit.min, rightMax);
  const pagesMax = Math.max(
    PANE_LIMITS.pages.min,
    Math.min(PANE_LIMITS.pages.max, room - rightWidth),
  );
  const pagesWidth = clamp(pagesRaw, PANE_LIMITS.pages.min, pagesMax);

  // Der Setter hängt nur an der Anordnung: gestapelt die rechte Spalte,
  // nebeneinander die Dateiliste.
  const setRightWidth = stacked ? setRight : setFiles;

  return {
    ref,
    layout,
    stacked,
    toggleLayout: () => setLayout(stacked ? "side" : "stacked"),
    pages: {
      width: pagesWidth,
      min: PANE_LIMITS.pages.min,
      max: pagesMax,
      def: PANE_LIMITS.pages.def,
      set: setPages,
    },
    right: {
      width: rightWidth,
      min: rightLimit.min,
      max: rightMax,
      def: rightLimit.def,
      set: setRightWidth,
    },
  };
}
