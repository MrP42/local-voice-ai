import React, {
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Check, X } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { TagDef } from "@/lib/tags/types";
import {
  TagAutocomplete,
  type TagAutocompleteHandle,
} from "../tags/TagAutocomplete";
import { TagContextMenu } from "../tags/TagContextMenu";

// ---------------------------------------------------------------------------
// BINDENDER Vertrag (Paket-Brief B-T3): Ein späteres Paket baut einen
// Sprecher-Provider gegen genau diese Schnittstellen. Felder nicht umbenennen,
// nichts Verpflichtendes ergänzen — optionale Erweiterungen sind erlaubt und
// als solche kommentiert.
// ---------------------------------------------------------------------------

/** Ein Fund eines Providers im Text. `start`/`end` sind UTF-16-Offsets
 *  (String-Indizes, wie `slice` sie nimmt), `raw` der exakte Ausschnitt. */
export interface ChipMatch {
  start: number;
  end: number;
  raw: string;
}

export interface ChipRenderSpec {
  label: string;
  /** CSS-Custom-Property-Name (ohne führendes `--`) einer Palettenfarbe —
   *  für Sprecher-Chips eines späteren Pakets. Ohne Token gilt das dezente
   *  Gelb der Tag-Chips. */
  colorToken?: string;
  state: "ok" | "invalid" | "suggestion";
  icon?: LucideIcon;
}

export interface ChipPopoverApi {
  close(): void;
  replaceRange(start: number, end: number, insert: string): void;
}

export interface ChipProvider {
  /** "tag" hier; "speaker" kommt in einem späteren Paket. */
  id: string;
  scan(text: string): ChipMatch[];
  render(m: ChipMatch): ChipRenderSpec;
  popover?(m: ChipMatch, api: ChipPopoverApi): React.ReactNode;
  /** Optionale Erweiterung: Meldungstext für `state:"invalid"`-Chips
   *  (landet in `onIssues`); ohne sie dient das Render-Label als Meldung. */
  issueMessage?(m: ChipMatch): string;
}

export interface ChipEditorIssue {
  providerId: string;
  match: ChipMatch;
  message: string;
}

/** Auto-Tagging-Vorschau (späteres Paket liefert sie): ein vorgeschlagenes
 *  Tag an einem Text-Offset, das noch NICHT im Text steht. */
export interface ChipEditorSuggestion {
  id: string;
  offset: number;
  tag: string;
}

export interface ChipEditorInsertApi {
  insertAtCursor(text: string): void;
  /** Optionale Erweiterung über den Vertrag hinaus (Palette-Drag):
   *  Einfügen an Viewport-Koordinaten. `false`, wenn der Punkt nicht über
   *  dem Editor liegt. Optionales Member, damit ein wörtlich gegen den
   *  Brief-Vertrag (`{ insertAtCursor }`) typisierter Consumer kompiliert —
   *  dieser Editor stellt es immer bereit. */
  insertAtPoint?(x: number, y: number, text: string): boolean;
}

interface TtsChipEditorProps {
  value: string;
  onChange: (t: string) => void;
  providers: ChipProvider[];
  /** Aggregiert alle `state:"invalid"`-Chips — damit die Seite Play/Export
   *  blockieren kann (Brief-Zusicherung 3). */
  onIssues?: (issues: ChipEditorIssue[]) => void;
  rows?: number;
  placeholder?: string;
  lang?: string;
  className?: string;
  insertApiRef?: React.Ref<ChipEditorInsertApi>;
  suggestions?: ChipEditorSuggestion[];
  onResolveSuggestion?: (id: string, accept: boolean) => void;
}

// ---------------------------------------------------------------------------
// Metrik-Deckungsgleichheit Mirror ↔ textarea
// ---------------------------------------------------------------------------

/**
 * Exakt die Klassen von `ui/Textarea` (Basis, dann Default-Variante, gleiche
 * Reihenfolge). Mirror und textarea tragen DENSELBEN String — wie auch immer
 * Tailwind die px-2/px-3-Doppelung der Vorlage auflöst, es löst sie für beide
 * gleich auf, und damit bleiben Padding, Font und Umbruch zeichenidentisch.
 * `whitespace-pre-wrap`/`break-words` sind für die textarea ohnehin die
 * UA-Defaults; explizit gesetzt gelten sie garantiert auch für den Mirror.
 */
const METRIC_CLASSES =
  "px-2 py-1 text-sm font-semibold px-3 py-2 text-start whitespace-pre-wrap break-words";

/** Sichtbarkeit/Interaktion der textarea — identisch zu `ui/Textarea`
 *  (default-Variante), plus w-full/block für den Overlay-Aufbau. */
const TEXTAREA_CLASSES =
  "block w-full bg-mid-gray/10 border border-mid-gray/80 rounded-md " +
  "transition-[background-color,border-color] duration-150 " +
  "hover:bg-logo-primary/10 hover:border-logo-primary " +
  "focus:outline-none focus:bg-logo-primary/10 focus:border-logo-primary " +
  "resize-y min-h-[100px] caret-text";

type AnchorRect = { left: number; top: number; bottom: number };

const toAnchor = (r: DOMRect): AnchorRect => ({
  left: r.left,
  top: r.top,
  bottom: r.bottom,
});

// ---------------------------------------------------------------------------
// Popover-Rahmen: fixiert, per Anker-Rect positioniert, an den Viewport
// geklemmt. Escape und Außenklick schließen. Keine Animationen — damit ist
// `prefers-reduced-motion` trivially erfüllt.
// ---------------------------------------------------------------------------

const AnchoredPopover: React.FC<{
  anchor: AnchorRect;
  onClose: () => void;
  ariaLabel: string;
  className?: string;
  children: React.ReactNode;
}> = ({ anchor, onClose, ariaLabel, className = "", children }) => {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const width = el.offsetWidth;
    const height = el.offsetHeight;
    const left = Math.min(
      Math.max(8, anchor.left),
      Math.max(8, window.innerWidth - width - 8),
    );
    let top = anchor.bottom + 4;
    if (top + height > window.innerHeight - 8) {
      top = Math.max(8, anchor.top - height - 4);
    }
    setPos({ left, top });
  }, [anchor]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    };
    const onPointerDown = (event: PointerEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) {
        onClose();
      }
    };
    // Scrollt die SEITE (nicht eine Liste im Popover selbst), stünde das
    // fixierte Popover an veralteten Viewport-Koordinaten — schließen.
    // capture, weil scroll-Ereignisse nicht bubbeln.
    const onScroll = (event: Event) => {
      if (
        ref.current &&
        event.target instanceof Node &&
        ref.current.contains(event.target)
      ) {
        return;
      }
      onClose();
    };
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("scroll", onScroll, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("scroll", onScroll, true);
    };
  }, [onClose]);

  return createPortal(
    <div
      ref={ref}
      role="dialog"
      aria-label={ariaLabel}
      tabIndex={-1}
      style={
        pos
          ? { left: pos.left, top: pos.top }
          : { left: -9999, top: 0, visibility: "hidden" }
      }
      className={`fixed z-50 rounded-lg border border-mid-gray/40 bg-background shadow-lg ${className}`}
    >
      {children}
    </div>,
    document.body,
  );
};

// ---------------------------------------------------------------------------
// Mirror-Segmente
// ---------------------------------------------------------------------------

interface RenderedMatch {
  providerId: string;
  match: ChipMatch;
  spec: ChipRenderSpec;
  hasPopover: boolean;
  message: string;
}

type Segment =
  | { kind: "text"; start: number; text: string }
  | { kind: "chip"; rendered: RenderedMatch }
  | { kind: "suggestion"; suggestion: ChipEditorSuggestion; offset: number }
  | { kind: "caret"; offset: number };

/** Chips färben den Originaltext ein, sie ersetzen ihn nicht: Hintergrund
 *  per Klasse, Rand per Outline (nach innen versetzt) — beides ohne
 *  Layout-Breite, damit die Metrik zeichenidentisch zur textarea bleibt. */
const chipStateClasses = (state: ChipRenderSpec["state"]): string => {
  switch (state) {
    case "invalid":
      return "bg-red-500/15 outline-solid outline-1 -outline-offset-1 outline-red-500/50";
    case "suggestion":
      return "bg-logo-primary/10 outline-dashed outline-1 -outline-offset-1 outline-logo-primary/60";
    default:
      // Dezentes Gelb + Ink — die TagChip-Stile aus A2, nur ohne Padding/Rand.
      return "bg-logo-primary/15 dark:bg-logo-primary/20 outline-solid outline-1 -outline-offset-1 outline-logo-primary/40";
  }
};

const chipColorStyle = (
  spec: ChipRenderSpec,
): React.CSSProperties | undefined =>
  spec.colorToken
    ? {
        backgroundColor: `color-mix(in srgb, var(--${spec.colorToken}) 18%, transparent)`,
        outlineColor: `color-mix(in srgb, var(--${spec.colorToken}) 45%, transparent)`,
      }
    : undefined;

type PopoverState =
  | { kind: "chip"; rendered: RenderedMatch; anchor: AnchorRect }
  | {
      kind: "suggestion";
      suggestion: ChipEditorSuggestion;
      anchor: AnchorRect;
    };

// ---------------------------------------------------------------------------
// Der Editor
// ---------------------------------------------------------------------------

/**
 * Mirror-Overlay-Editor: Die native textarea bleibt die einzige Wahrheit
 * (Undo/IME/RTL/Screenreader nativ); ein zeichenidentischer Mirror liegt
 * darüber und färbt Provider-Funde als Chips ein. Der Mirror ist
 * `pointer-events: none` — nur Chip-Spans (und Vorschlags-Anker) fangen
 * Zeigerereignisse.
 */
export const TtsChipEditor: React.FC<TtsChipEditorProps> = ({
  value,
  onChange,
  providers,
  onIssues,
  rows,
  placeholder,
  lang,
  className = "",
  insertApiRef,
  suggestions,
  onResolveSuggestion,
}) => {
  const { t } = useTranslation();
  const taRef = useRef<HTMLTextAreaElement>(null);
  const mirrorRef = useRef<HTMLDivElement>(null);

  const valueRef = useRef(value);
  useEffect(() => {
    valueRef.current = value;
  }, [value]);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  const [composing, setComposing] = useState(false);
  const composingRef = useRef(false);

  const [popover, setPopover] = useState<PopoverState | null>(null);

  /** Autocomplete: Caret steht hinter einem ungeschlossenen `[`. */
  const [ac, setAc] = useState<{ bracket: number; caret: number } | null>(null);
  const [acRect, setAcRect] = useState<AnchorRect | null>(null);
  const acHandleRef = useRef<TagAutocompleteHandle>(null);
  /** Per Escape verworfen — für DIESE Klammer nicht wieder öffnen. */
  const dismissedRef = useRef<number | null>(null);

  /** Kontextmenü: Ort + die Selektion zum Zeitpunkt des Rechtsklicks. */
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    selStart: number;
    selEnd: number;
  } | null>(null);

  /** Zwingt Anker-Neuvermessung (Scroll/Resize) ohne Layout-Umbau. */
  const [tick, setTick] = useState(0);

  // ---- Scan + Überlappungsregel (Brief-Zusicherung 1) --------------------

  const rendered = useMemo<RenderedMatch[]>(() => {
    const all: Array<{ providerId: string; match: ChipMatch }> = [];
    for (const provider of providers) {
      for (const match of provider.scan(value)) {
        all.push({ providerId: provider.id, match });
      }
    }
    // Stabil nach Start sortiert; bei Überlapp gewinnt der frühere Start
    // (bei gleichem Start der zuerst gelistete Provider).
    all.sort((a, b) => a.match.start - b.match.start);
    const result: RenderedMatch[] = [];
    let lastEnd = 0;
    for (const entry of all) {
      if (entry.match.start < lastEnd && result.length > 0) continue;
      const provider = providers.find((p) => p.id === entry.providerId);
      if (!provider) continue;
      const spec = provider.render(entry.match);
      result.push({
        providerId: entry.providerId,
        match: entry.match,
        spec,
        hasPopover: typeof provider.popover === "function",
        message:
          spec.state === "invalid"
            ? (provider.issueMessage?.(entry.match) ?? spec.label)
            : "",
      });
      lastEnd = entry.match.end;
    }
    return result;
  }, [providers, value]);

  // ---- onIssues (Brief-Zusicherung 3) ------------------------------------

  const issues = useMemo<ChipEditorIssue[]>(
    () =>
      rendered
        .filter((r) => r.spec.state === "invalid")
        .map((r) => ({
          providerId: r.providerId,
          match: r.match,
          message: r.message,
        })),
    [rendered],
  );
  const issuesKey = issues
    .map((i) => `${i.providerId}:${i.match.start}:${i.match.end}:${i.message}`)
    .join("|");
  const lastIssuesKey = useRef<string | null>(null);
  useEffect(() => {
    if (!onIssues) return;
    if (lastIssuesKey.current === issuesKey) return;
    lastIssuesKey.current = issuesKey;
    onIssues(issues);
    // issuesKey deckt den Inhalt von issues vollständig ab.
  }, [issuesKey, onIssues]);

  // ---- Einfügen mit nativer Undo-Historie (Brief-Zusicherung 2) ----------

  /**
   * Einfügen/Ersetzen über `document.execCommand('insertText')` in der
   * fokussierten textarea, damit Strg+Z funktioniert; Löschen über
   * `execCommand('delete')` (insertText mit Leerstring meldet Chromium als
   * Fehlschlag). Fallback: manueller Offset-Splice per setState — dann ohne
   * nativen Undo-Schritt.
   */
  const execInsert = useCallback(
    (ta: HTMLTextAreaElement, start: number, end: number, insert: string) => {
      if (insert === "" && start === end) return;
      ta.focus();
      ta.setSelectionRange(start, end);
      let ok = false;
      try {
        ok =
          insert === ""
            ? document.execCommand("delete")
            : document.execCommand("insertText", false, insert);
      } catch {
        ok = false;
      }
      if (!ok) {
        const current = valueRef.current;
        const next = current.slice(0, start) + insert + current.slice(end);
        valueRef.current = next;
        onChangeRef.current(next);
        const caret = start + insert.length;
        window.requestAnimationFrame(() => {
          ta.setSelectionRange(caret, caret);
        });
      }
    },
    [],
  );

  const replaceRange = useCallback(
    (start: number, end: number, insert: string) => {
      const ta = taRef.current;
      if (!ta) return;
      execInsert(ta, start, end, insert);
    },
    [execInsert],
  );

  const insertAtCursor = useCallback(
    (text: string) => {
      const ta = taRef.current;
      if (!ta) return;
      execInsert(ta, ta.selectionStart, ta.selectionEnd, text);
    },
    [execInsert],
  );

  // ---- Punkt → Offset (Palette-Drag) -------------------------------------

  const offsetFromPoint = useCallback((x: number, y: number): number | null => {
    const mir = mirrorRef.current;
    if (!mir) return null;
    const rect = mir.getBoundingClientRect();
    if (x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) {
      return null;
    }
    // caretRangeFromPoint überspringt pointer-events:none-Elemente — den
    // Mirror für die Messung kurz anfassbar machen.
    const prev = mir.style.pointerEvents;
    mir.style.pointerEvents = "auto";
    let node: Node | null = null;
    let nodeOffset = 0;
    try {
      type CaretPoint = { offsetNode: Node; offset: number };
      const doc = document as Document & {
        caretRangeFromPoint?: (x: number, y: number) => Range | null;
        caretPositionFromPoint?: (x: number, y: number) => CaretPoint | null;
      };
      if (typeof doc.caretRangeFromPoint === "function") {
        const range = doc.caretRangeFromPoint(x, y);
        if (range) {
          node = range.startContainer;
          nodeOffset = range.startOffset;
        }
      } else if (typeof doc.caretPositionFromPoint === "function") {
        const posAt = doc.caretPositionFromPoint(x, y);
        if (posAt) {
          node = posAt.offsetNode;
          nodeOffset = posAt.offset;
        }
      }
    } finally {
      mir.style.pointerEvents = prev;
    }
    const length = valueRef.current.length;
    if (!node || !mir.contains(node)) return length;
    const el =
      node.nodeType === Node.TEXT_NODE ? node.parentElement : (node as Element);
    const span = el?.closest?.("[data-off]") ?? null;
    if (!span || !mir.contains(span)) return length;
    const base = Number(span.getAttribute("data-off"));
    if (Number.isNaN(base)) return length;
    // Vorschlags-Anker: nur der Basis-Offset zählt (Inhalt ist schwebend).
    if (span.getAttribute("data-anchor") === "1") return base;
    if (node.nodeType !== Node.TEXT_NODE) return base;
    // Mitten in einem Chip landet nichts: an die nähere Kante schnappen,
    // damit ein Drop nie ein Tag zerreißt.
    if (span.getAttribute("data-chip") === "1") {
      const rawLength = span.textContent?.length ?? 0;
      return nodeOffset < rawLength / 2 ? base : base + rawLength;
    }
    return Math.min(base + nodeOffset, length);
  }, []);

  const insertAtPoint = useCallback(
    (x: number, y: number, text: string): boolean => {
      const ta = taRef.current;
      if (!ta) return false;
      const offset = offsetFromPoint(x, y);
      if (offset === null) return false;
      execInsert(ta, offset, offset, text);
      return true;
    },
    [execInsert, offsetFromPoint],
  );

  useImperativeHandle(insertApiRef, () => ({ insertAtCursor, insertAtPoint }), [
    insertAtCursor,
    insertAtPoint,
  ]);

  // ---- Autocomplete-Erkennung (onChange + selectionchange) ---------------

  const detectAutocomplete = useCallback(() => {
    const ta = taRef.current;
    if (
      !ta ||
      composingRef.current ||
      document.activeElement !== ta ||
      ta.selectionStart !== ta.selectionEnd
    ) {
      setAc(null);
      return;
    }
    const caret = ta.selectionStart;
    const text = valueRef.current;
    let bracket = -1;
    for (let i = caret - 1; i >= 0; i--) {
      const c = text[i];
      if (c === "[") {
        bracket = i;
        break;
      }
      if (c === "]" || c === "\n") break;
    }
    if (bracket === -1) {
      dismissedRef.current = null;
      setAc(null);
      return;
    }
    // "Ungeschlossen" heißt: vorwärts bis Zeilenende kommt kein `]`, bevor
    // eine neue Klammer beginnt. Steht der Caret in einem fertigen Tag,
    // ist das Popover zuständig, nicht das Autocomplete.
    for (let i = caret; i < text.length; i++) {
      const c = text[i];
      if (c === "]") {
        setAc(null);
        return;
      }
      if (c === "[" || c === "\n") break;
    }
    if (dismissedRef.current === bracket) {
      setAc(null);
      return;
    }
    setAc((prev) =>
      prev && prev.bracket === bracket && prev.caret === caret
        ? prev
        : { bracket, caret },
    );
  }, []);

  useEffect(() => {
    const handler = () => detectAutocomplete();
    document.addEventListener("selectionchange", handler);
    return () => document.removeEventListener("selectionchange", handler);
  }, [detectAutocomplete]);

  // Scrollt die SEITE (oder irgendein Container darüber), wandert der
  // Caret-Anker im Viewport mit — das offene Autocomplete misst neu
  // (capture, weil scroll-Ereignisse nicht bubbeln). Die eigene textarea
  // ist über handleScroll ohnehin abgedeckt; ein doppeltes tick++ ist
  // harmlos, der Rect-Vergleich verhindert Zustands-Flattern.
  useEffect(() => {
    if (!ac) return;
    const onAnyScroll = () => setTick((n) => n + 1);
    document.addEventListener("scroll", onAnyScroll, true);
    return () => document.removeEventListener("scroll", onAnyScroll, true);
  }, [ac]);

  // ---- Mirror-Segmente ----------------------------------------------------

  const segments = useMemo<Segment[]>(() => {
    const clamp = (n: number) => Math.max(0, Math.min(n, value.length));
    const anchors: Array<{ offset: number; seg: Segment }> = [];
    for (const s of suggestions ?? []) {
      let offset = clamp(s.offset);
      // Fällt ein Vorschlag mitten in einen Chip, rückt er an dessen Anfang.
      const covering = rendered.find(
        (r) => r.match.start < offset && offset < r.match.end,
      );
      if (covering) offset = covering.match.start;
      anchors.push({
        offset,
        seg: { kind: "suggestion", suggestion: s, offset },
      });
    }
    if (ac) {
      const offset = clamp(ac.caret);
      anchors.push({ offset, seg: { kind: "caret", offset } });
    }
    anchors.sort((a, b) => a.offset - b.offset);

    const out: Segment[] = [];
    let ai = 0;
    const emitText = (from: number, to: number) => {
      let cur = from;
      while (ai < anchors.length && anchors[ai].offset <= to) {
        const { offset, seg } = anchors[ai];
        if (offset >= from) {
          if (offset > cur) {
            out.push({
              kind: "text",
              start: cur,
              text: value.slice(cur, offset),
            });
            cur = offset;
          }
          out.push(seg);
        }
        ai++;
      }
      if (to > cur) {
        out.push({ kind: "text", start: cur, text: value.slice(cur, to) });
      }
    };
    let pos = 0;
    for (const r of rendered) {
      emitText(pos, r.match.start);
      out.push({ kind: "chip", rendered: r });
      pos = r.match.end;
    }
    emitText(pos, value.length);
    return out;
  }, [value, rendered, suggestions, ac]);

  // ---- Geometrie- und Scroll-Sync ----------------------------------------

  /**
   * Der Mirror wird auf die CLIENT-Fläche der textarea gelegt (innerhalb des
   * Rands, ohne Scrollleiste): `clientWidth` schrumpft mit erscheinender
   * Scrollleiste, damit bricht der Mirror exakt wie die textarea um. In RTL
   * legt Chromium die Scrollleiste links an — dann verschiebt sich die
   * Client-Fläche entsprechend.
   */
  const syncGeometry = useCallback(() => {
    const ta = taRef.current;
    const mir = mirrorRef.current;
    if (!ta || !mir) return;
    const cs = getComputedStyle(ta);
    const borderLeft = parseFloat(cs.borderLeftWidth) || 0;
    const borderRight = parseFloat(cs.borderRightWidth) || 0;
    const borderTop = parseFloat(cs.borderTopWidth) || 0;
    const scrollbar =
      ta.offsetWidth - ta.clientWidth - borderLeft - borderRight;
    const left =
      cs.direction === "rtl"
        ? ta.offsetLeft + borderLeft + Math.max(0, scrollbar)
        : ta.offsetLeft + borderLeft;
    mir.style.top = `${ta.offsetTop + borderTop}px`;
    mir.style.left = `${left}px`;
    mir.style.width = `${ta.clientWidth}px`;
    mir.style.height = `${ta.clientHeight}px`;
    mir.scrollTop = ta.scrollTop;
    mir.scrollLeft = ta.scrollLeft;
  }, []);

  // Nach jedem Render: billig, und deckt Wert-, Layout- und Themewechsel ab.
  useLayoutEffect(() => {
    syncGeometry();
  });

  useEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    const ro = new ResizeObserver(() => {
      syncGeometry();
      setPopover(null);
      setTick((n) => n + 1);
    });
    ro.observe(ta);
    return () => ro.disconnect();
  }, [syncGeometry]);

  const handleScroll = () => {
    const ta = taRef.current;
    const mir = mirrorRef.current;
    if (ta && mir) {
      mir.scrollTop = ta.scrollTop;
      mir.scrollLeft = ta.scrollLeft;
    }
    // Chip-/Vorschlags-Popover hätte einen veralteten Anker — schließen;
    // das Autocomplete folgt dem Caret über die Neuvermessung.
    setPopover(null);
    if (ac) setTick((n) => n + 1);
  };

  // ---- Autocomplete-Anker (Caret-Span im Mirror) -------------------------

  useLayoutEffect(() => {
    if (!ac) {
      setAcRect(null);
      return;
    }
    const el = mirrorRef.current?.querySelector("[data-caret]");
    if (!el) {
      setAcRect(null);
      return;
    }
    const r = (el as HTMLElement).getBoundingClientRect();
    setAcRect((prev) =>
      prev &&
      Math.abs(prev.left - r.left) < 0.5 &&
      Math.abs(prev.top - r.top) < 0.5 &&
      Math.abs(prev.bottom - r.bottom) < 0.5
        ? prev
        : toAnchor(r),
    );
    // tick erzwingt Neuvermessung nach Scroll/Resize.
  }, [ac, value, tick]);

  // ---- Ereignisse ---------------------------------------------------------

  const handleChange = (event: React.ChangeEvent<HTMLTextAreaElement>) => {
    valueRef.current = event.target.value;
    onChange(event.target.value);
    detectAutocomplete();
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (ac && acRect && acHandleRef.current?.handleKey(event.key)) {
      event.preventDefault();
    }
  };

  const handleCompositionStart = () => {
    composingRef.current = true;
    setComposing(true);
    setAc(null);
  };

  const handleCompositionEnd = () => {
    composingRef.current = false;
    setComposing(false);
    detectAutocomplete();
  };

  const handleContextMenu = (event: React.MouseEvent<HTMLTextAreaElement>) => {
    event.preventDefault();
    const ta = taRef.current;
    if (!ta) return;
    let x = event.clientX;
    let y = event.clientY;
    if (x === 0 && y === 0) {
      // Menü-Taste der Tastatur: neben dem Feldanfang öffnen.
      const r = ta.getBoundingClientRect();
      x = r.left + 16;
      y = r.top + 16;
    }
    setMenu({ x, y, selStart: ta.selectionStart, selEnd: ta.selectionEnd });
  };

  // preventScroll: Ein Schließen durch Seiten-Scroll darf den Viewport
  // nicht zurück zur textarea reißen.
  const closePopover = useCallback(() => {
    setPopover(null);
    taRef.current?.focus({ preventScroll: true });
  }, []);

  const closeMenu = useCallback(() => {
    setMenu(null);
    taRef.current?.focus({ preventScroll: true });
  }, []);

  // Textänderungen machen gespeicherte Match-Offsets ungültig — Popover zu.
  useEffect(() => {
    setPopover(null);
  }, [value]);

  const openChipPopover = (r: RenderedMatch, el: HTMLElement) => {
    if (!r.hasPopover) return;
    setPopover({
      kind: "chip",
      rendered: r,
      anchor: toAnchor(el.getBoundingClientRect()),
    });
  };

  const openSuggestionPopover = (
    suggestion: ChipEditorSuggestion,
    el: HTMLElement,
  ) => {
    setPopover({
      kind: "suggestion",
      suggestion,
      anchor: toAnchor(el.getBoundingClientRect()),
    });
  };

  // ---- Kontextmenü-Grundfunktionen (Offset-Splice + navigator.clipboard) --

  const menuCut = () => {
    if (!menu) return;
    const { selStart, selEnd } = menu;
    const selected = valueRef.current.slice(selStart, selEnd);
    closeMenu();
    if (!selected) return;
    void navigator.clipboard?.writeText(selected).catch(() => undefined);
    const ta = taRef.current;
    if (ta) execInsert(ta, selStart, selEnd, "");
  };

  const menuCopy = () => {
    if (!menu) return;
    const selected = valueRef.current.slice(menu.selStart, menu.selEnd);
    closeMenu();
    if (!selected) return;
    void navigator.clipboard?.writeText(selected).catch(() => undefined);
  };

  const menuPaste = () => {
    if (!menu) return;
    const { selStart, selEnd } = menu;
    closeMenu();
    void navigator.clipboard
      ?.readText()
      .then((clipText) => {
        const ta = taRef.current;
        if (!clipText || !ta) return;
        execInsert(ta, selStart, selEnd, clipText);
      })
      .catch(() => undefined);
  };

  const menuInsertTag = (tagText: string) => {
    if (!menu) return;
    const { selStart, selEnd } = menu;
    closeMenu();
    const ta = taRef.current;
    if (ta) execInsert(ta, selStart, selEnd, tagText);
  };

  // ---- Render -------------------------------------------------------------

  const provider =
    popover?.kind === "chip"
      ? providers.find((p) => p.id === popover.rendered.providerId)
      : undefined;

  return (
    <div className={`relative ${className}`} lang={lang}>
      <textarea
        ref={taRef}
        value={value}
        onChange={handleChange}
        onScroll={handleScroll}
        onKeyDown={handleKeyDown}
        onCompositionStart={handleCompositionStart}
        onCompositionEnd={handleCompositionEnd}
        onContextMenu={handleContextMenu}
        onBlur={() => setAc(null)}
        placeholder={placeholder}
        rows={rows}
        lang={lang}
        className={`${METRIC_CLASSES} ${TEXTAREA_CLASSES} ${
          // Text unsichtbar, Caret und Selektion sichtbar. Während einer
          // IME-Komposition (und im Leerzustand, damit der Placeholder
          // erscheint) bleibt der native Text sichtbar.
          composing || value.length === 0
            ? ""
            : "[-webkit-text-fill-color:transparent]"
        }`}
      />
      <div
        ref={mirrorRef}
        aria-hidden="true"
        className={`${METRIC_CLASSES} pointer-events-none absolute overflow-hidden ${
          composing ? "invisible" : ""
        }`}
      >
        {segments.map((seg, index) => {
          switch (seg.kind) {
            case "text":
              return (
                <span key={index} data-off={seg.start}>
                  {seg.text}
                </span>
              );
            case "chip": {
              const { match, spec, hasPopover } = seg.rendered;
              return (
                <span
                  key={index}
                  data-off={match.start}
                  data-chip="1"
                  title={spec.label}
                  onClick={(event) =>
                    openChipPopover(seg.rendered, event.currentTarget)
                  }
                  className={`pointer-events-auto rounded-sm ${
                    hasPopover ? "cursor-pointer" : ""
                  } ${chipStateClasses(spec.state)}`}
                  style={chipColorStyle(spec)}
                >
                  {match.raw}
                </span>
              );
            }
            case "suggestion":
              // Nullbreiter Anker: der gestrichelte Vorschlags-Chip schwebt
              // über der Zeile und nimmt keinen Platz im Textfluss ein —
              // sonst wären Mirror und textarea nicht mehr deckungsgleich.
              return (
                <span
                  key={`s:${seg.suggestion.id}`}
                  data-off={seg.offset}
                  data-anchor="1"
                  className="relative inline-block h-0 w-0 align-baseline"
                >
                  <button
                    type="button"
                    tabIndex={-1}
                    title={t("tts.editor.suggestionAria")}
                    onClick={(event) =>
                      openSuggestionPopover(seg.suggestion, event.currentTarget)
                    }
                    className="pointer-events-auto absolute bottom-full left-0 z-10 -translate-y-0.5 cursor-pointer whitespace-nowrap rounded border border-dashed border-logo-primary/60 bg-background px-1 text-[10px] leading-4 text-text/80"
                  >
                    [{seg.suggestion.tag}]
                  </button>
                </span>
              );
            case "caret":
              return (
                <span
                  key="caret"
                  data-caret="1"
                  data-off={seg.offset}
                  className="inline-block w-0"
                />
              );
          }
        })}
        {/* Sentinel: gibt einer leeren Schlusszeile (Text endet mit \n)
            dieselbe Höhe wie in der textarea — sonst liefe der Scroll-Sync
            am Textende auseinander. */}
        {"​"}
      </div>

      {ac && acRect && !composing && (
        <TagAutocomplete
          ref={acHandleRef}
          anchor={acRect}
          query={value.slice(ac.bracket + 1, ac.caret)}
          onPick={(def: TagDef) => {
            const current = ac;
            dismissedRef.current = null;
            setAc(null);
            const ta = taRef.current;
            if (ta) {
              execInsert(
                ta,
                current.bracket + 1,
                current.caret,
                `${def.insert}]`,
              );
            }
          }}
          onDismiss={() => {
            dismissedRef.current = ac.bracket;
            setAc(null);
          }}
        />
      )}

      {popover?.kind === "chip" && provider?.popover && (
        <AnchoredPopover
          anchor={popover.anchor}
          onClose={closePopover}
          ariaLabel={popover.rendered.spec.label}
        >
          {provider.popover(popover.rendered.match, {
            close: closePopover,
            replaceRange,
          })}
        </AnchoredPopover>
      )}

      {popover?.kind === "suggestion" && (
        <AnchoredPopover
          anchor={popover.anchor}
          onClose={closePopover}
          ariaLabel={t("tts.editor.suggestionAria")}
        >
          <div className="flex items-center gap-1 p-1">
            <span className="px-2 text-sm font-medium text-text">
              [{popover.suggestion.tag}]
            </span>
            <button
              type="button"
              onClick={() => {
                const id = popover.suggestion.id;
                closePopover();
                onResolveSuggestion?.(id, true);
              }}
              title={t("tts.editor.suggestionAccept")}
              aria-label={t("tts.editor.suggestionAccept")}
              className="flex min-h-[44px] min-w-[44px] cursor-pointer items-center justify-center rounded-md text-text/70 transition-colors hover:bg-logo-primary/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
            >
              <Check width={16} height={16} aria-hidden="true" />
            </button>
            <button
              type="button"
              onClick={() => {
                const id = popover.suggestion.id;
                closePopover();
                onResolveSuggestion?.(id, false);
              }}
              title={t("tts.editor.suggestionReject")}
              aria-label={t("tts.editor.suggestionReject")}
              className="flex min-h-[44px] min-w-[44px] cursor-pointer items-center justify-center rounded-md text-text/70 transition-colors hover:bg-mid-gray/20 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
            >
              <X width={16} height={16} aria-hidden="true" />
            </button>
          </div>
        </AnchoredPopover>
      )}

      {menu && (
        <TagContextMenu
          x={menu.x}
          y={menu.y}
          hasSelection={menu.selEnd > menu.selStart}
          onClose={closeMenu}
          onCut={menuCut}
          onCopy={menuCopy}
          onPaste={menuPaste}
          onInsertTag={menuInsertTag}
        />
      )}
    </div>
  );
};
