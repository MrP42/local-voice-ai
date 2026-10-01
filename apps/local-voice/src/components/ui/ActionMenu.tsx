import React, {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import type { LucideIcon } from "lucide-react";
import { IconAction, type IconActionProps } from "./IconAction";

// Menü und Popover an einem Symbol-Knopf. Beide werden per Portal nach
// document.body gehängt und mit position: fixed aus dem Rechteck des Knopfes
// platziert. Im Dokumentfluss (absolut) würde die scrollende, in der Höhe
// gedeckelte Bedienspalte sie abschneiden: das Menü wäre im DOM "sichtbar",
// aber zum Teil nicht erreichbar. Der Preis: die Tab-Reihenfolge folgt nicht
// mehr dem Knopf; dafür landet der Fokus beim Öffnen im Menü und kehrt beim
// Schließen zum Knopf zurück.

/** Der Knopf, an dem Menü oder Popover hängen. */
type TriggerProps = Pick<
  IconActionProps,
  | "icon"
  | "iconClassName"
  | "label"
  | "description"
  | "testId"
  | "wrapperTestId"
  | "badge"
  | "badgeTestId"
  | "disabled"
  | "size"
>;

export interface ActionMenuItem {
  id: string;
  label: string;
  icon: LucideIcon;
  iconClassName?: string;
  onSelect: () => void;
  disabled?: boolean;
  testId?: string;
  /** Längere Erklärung als natives Tooltip des Eintrags. */
  title?: string;
  /** Rechts im Eintrag, z. B. der Fehlerzähler. */
  trailing?: React.ReactNode;
}

interface ActionMenuProps {
  trigger: TriggerProps;
  items: ActionMenuItem[];
  menuLabel: string;
  /** Kante des Knopfes, an der das Menü bündig ansetzt. */
  align?: "start" | "end";
  /** Tailwind-Breite, Standard w-64. */
  widthClass?: string;
}

/** Schließen bei Klick daneben (mousedown, damit ein Klick auf einen
 *  anderen Knopf nicht erst nach dem Schließen ankommt). Knopf und Fenster
 *  liegen wegen des Portals an verschiedenen Stellen im DOM: als "innen"
 *  gilt beides. */
const useOutsideClose = (
  refs: React.RefObject<HTMLElement>[],
  open: boolean,
  onClose: () => void,
) => {
  const refsRef = useRef(refs);
  refsRef.current = refs;
  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      const target = event.target as Node;
      // Auswahllisten mit `menuPortal` hängen an body, nicht im Fenster: ein
      // Klick darauf ist kein Klick daneben.
      const inside =
        refsRef.current.some((r) => r.current?.contains(target)) ||
        (target instanceof Element &&
          target.closest(SELECT_MENU_PORTAL) !== null);
      if (!inside) onClose();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open, onClose]);
};

const SELECT_MENU_PORTAL = ".app-select__menu-portal";

/** Ist im Popover eine Auswahlliste offen (im Fenster oder als Portal)? */
const hasOpenSelectMenu = (panel: HTMLElement | null) =>
  !!panel?.querySelector(".app-select__menu") ||
  !!document.querySelector(`${SELECT_MENU_PORTAL} .app-select__menu`);

const MARGIN = 8;
const GAP = 4;

interface Placement {
  top: number;
  left: number;
  width?: number;
  maxHeight: number;
  /** Nur scrollen, wenn der Inhalt nicht passt: sonst würde ein aufklappendes
   *  Auswahlmenü im Popover abgeschnitten. */
  scroll: boolean;
}

/**
 * Platziert ein fixiertes Fenster unter dem Knopf (nach oben, wenn unten kein
 * Platz ist) und hält es horizontal im Fenster. Bis zur ersten Messung ist es
 * unsichtbar, damit es nicht an (0, 0) aufblitzt. Bei Scroll (auch in
 * verschachtelten Containern) und Größenänderung wird neu gerechnet.
 * `rowRef` (optional) liefert die Breite und linke Kante, an der das Fenster
 * ausgerichtet wird (Popover: ganze Zeile statt nur der Knopf).
 */
const useAnchoredPlacement = (
  open: boolean,
  anchorRef: React.RefObject<HTMLElement>,
  panelRef: React.RefObject<HTMLElement>,
  align: "start" | "end",
  rowRef?: React.RefObject<HTMLElement | null>,
) => {
  const [placement, setPlacement] = useState<Placement | null>(null);

  useLayoutEffect(() => {
    if (!open) {
      setPlacement(null);
      return;
    }
    const compute = () => {
      const anchor = anchorRef.current;
      const panel = panelRef.current;
      if (!anchor || !panel) return;
      const a = anchor.getBoundingClientRect();
      const row = rowRef?.current?.getBoundingClientRect();
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      const width = row ? Math.min(row.width, vw - 2 * MARGIN) : undefined;
      // Breite zuerst setzen: die Höhe hängt vom Umbruch ab.
      if (width !== undefined) panel.style.width = `${width}px`;
      const panelW = width ?? panel.offsetWidth;
      // Natürliche Höhe, auch wenn maxHeight sie schon deckelt.
      const panelH =
        panel.scrollHeight + (panel.offsetHeight - panel.clientHeight);

      const below = vh - a.bottom - GAP - MARGIN;
      const above = a.top - GAP - MARGIN;
      // Nach oben klappen, wenn unten nicht genug Platz ist und oben mehr.
      const flip = panelH > below && above > below;
      const room = Math.max(flip ? above : below, 80);
      const scroll = panelH > room;
      const height = Math.min(panelH, room);
      const top = flip ? a.top - GAP - height : a.bottom + GAP;

      const rtl =
        getComputedStyle(document.documentElement).direction === "rtl";
      const alignRight = (align === "end") !== rtl;
      const edgeLeft = row ? row.left : a.left;
      const edgeRight = row ? row.right : a.right;
      let left = alignRight ? edgeRight - panelW : edgeLeft;
      left = Math.max(MARGIN, Math.min(left, vw - panelW - MARGIN));

      setPlacement((old) =>
        old &&
        old.top === top &&
        old.left === left &&
        old.width === width &&
        old.maxHeight === room &&
        old.scroll === scroll
          ? old
          : { top, left, width, maxHeight: room, scroll },
      );
    };
    compute();
    // Scroll in jedem Vorfahren (Bedienspalte, Seite) verschiebt den Knopf.
    window.addEventListener("scroll", compute, true);
    window.addEventListener("resize", compute);
    return () => {
      window.removeEventListener("scroll", compute, true);
      window.removeEventListener("resize", compute);
    };
  }, [open, anchorRef, panelRef, align, rowRef]);

  const style: React.CSSProperties = placement
    ? {
        position: "fixed",
        top: placement.top,
        left: placement.left,
        width: placement.width,
        maxHeight: placement.maxHeight,
        overflowY: placement.scroll ? "auto" : undefined,
      }
    : { position: "fixed", top: 0, left: 0, visibility: "hidden" };
  return { style, placed: placement !== null };
};

const MENU_CLASSES =
  "z-40 rounded-lg border border-mid-gray/40 bg-background py-1 shadow-lg";

export const ActionMenu: React.FC<ActionMenuProps> = ({
  trigger,
  items,
  menuLabel,
  align = "start",
  widthClass = "w-64",
}) => {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuId = useId();

  const focusables = () =>
    Array.from(
      menuRef.current?.querySelectorAll<HTMLButtonElement>(
        '[role="menuitem"]:not(:disabled)',
      ) ?? [],
    );

  const close = (returnFocus: boolean) => {
    setOpen(false);
    // Vor dem Neuzeichnen fokussieren: solange das Menü noch offen ist,
    // hält der Tooltip des Knopfes still.
    if (returnFocus) buttonRef.current?.focus();
  };
  const closeOutside = React.useCallback(() => setOpen(false), []);
  useOutsideClose([wrapRef, menuRef], open, closeOutside);
  const { style: placementStyle, placed } = useAnchoredPlacement(
    open,
    buttonRef,
    menuRef,
    align,
  );

  // Beim Öffnen landet der Fokus im Menü: erster Eintrag (mit Pfeil nach oben
  // vom Knopf aus: letzter). Erst nach der Platzierung: ein unsichtbares
  // Fenster nimmt keinen Fokus an, und preventScroll hält die Bedienspalte still.
  const focusTarget = useRef<"first" | "last">("first");
  useEffect(() => {
    if (!open || !placed) return;
    const list = focusables();
    (focusTarget.current === "last" ? list[list.length - 1] : list[0])?.focus({
      preventScroll: true,
    });
    focusTarget.current = "first";
  }, [open, placed]);

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (!open) return;
    const list = focusables();
    const index = list.indexOf(document.activeElement as HTMLButtonElement);
    switch (event.key) {
      case "Escape":
        event.preventDefault();
        close(true);
        return;
      case "Tab":
        // Das Menü verlässt man nicht mit Tab ins Leere: schließen, Fokus
        // zurück zum Knopf, der Nutzer tabbt von dort weiter.
        event.preventDefault();
        close(true);
        return;
      case "ArrowDown":
        if (list.length === 0) return;
        event.preventDefault();
        list[(index + 1) % list.length]?.focus();
        return;
      case "ArrowUp":
        if (list.length === 0) return;
        event.preventDefault();
        list[(index - 1 + list.length) % list.length]?.focus();
        return;
      case "Home":
        event.preventDefault();
        list[0]?.focus();
        return;
      case "End":
        event.preventDefault();
        list[list.length - 1]?.focus();
        return;
    }
  };

  return (
    <div ref={wrapRef} className="relative" onKeyDown={onKeyDown}>
      <IconAction
        ref={buttonRef}
        {...trigger}
        suppressTooltip={open}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        onClick={() => setOpen((o) => !o)}
        onKeyDown={(event) => {
          // Pfeil nach unten/oben öffnet wie bei jedem Menüknopf.
          if (!open && event.key === "ArrowDown") {
            event.preventDefault();
            focusTarget.current = "first";
            setOpen(true);
          } else if (!open && event.key === "ArrowUp") {
            event.preventDefault();
            focusTarget.current = "last";
            setOpen(true);
          }
        }}
      />
      {open &&
        createPortal(
          <div
            ref={menuRef}
            id={menuId}
            role="menu"
            aria-label={menuLabel}
            style={placementStyle}
            className={`${MENU_CLASSES} ${widthClass}`}
          >
            {items.map((item) => {
              const Icon = item.icon;
              return (
                <button
                  key={item.id}
                  type="button"
                  role="menuitem"
                  tabIndex={-1}
                  disabled={item.disabled}
                  title={item.title}
                  data-testid={item.testId}
                  onClick={() => {
                    close(true);
                    item.onSelect();
                  }}
                  className="flex w-full cursor-pointer items-center gap-2 px-3 py-2 text-start text-sm text-text/80 hover:bg-mid-gray/15 hover:text-text focus:bg-mid-gray/15 focus:text-text focus:outline-none disabled:cursor-not-allowed disabled:opacity-50"
                >
                  <Icon
                    width={16}
                    height={16}
                    aria-hidden="true"
                    className={item.iconClassName}
                  />
                  <span className="flex-1">{item.label}</span>
                  {item.trailing}
                </button>
              );
            })}
          </div>,
          document.body,
        )}
    </div>
  );
};

interface ActionPopoverProps {
  trigger: TriggerProps;
  /** Name des Popovers für Screenreader. */
  popoverLabel: string;
  popoverTestId?: string;
  children: React.ReactNode;
}

/**
 * Popover an einem Symbol-Knopf für Einstellungen, die man selten anfasst
 * (z. B. Umfang, Detailgrad, Zielgruppe). Nimmt die Breite der Zeile, in der
 * der Knopf steht, damit es nicht über die Spalte hinausragt.
 */
export const ActionPopover: React.FC<ActionPopoverProps> = ({
  trigger,
  popoverLabel,
  popoverTestId,
  children,
}) => {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const panelId = useId();
  // Die Zeile, in der der Knopf steht (der Wrapper selbst ist "contents").
  const rowRef = useRef<HTMLElement | null>(null);
  useLayoutEffect(() => {
    rowRef.current = wrapRef.current?.parentElement ?? null;
  }, [open]);
  const closeOutside = React.useCallback(() => setOpen(false), []);
  useOutsideClose([wrapRef, panelRef], open, closeOutside);
  const { style: placementStyle, placed } = useAnchoredPlacement(
    open,
    buttonRef,
    panelRef,
    "start",
    rowRef,
  );

  useEffect(() => {
    if (!open || !placed) return;
    panelRef.current
      ?.querySelector<HTMLElement>("input,button,select,textarea,[tabindex]")
      ?.focus({ preventScroll: true });
  }, [open, placed]);

  // Das Fenster hängt am Ende von body: Tab am Rand würde die Seite verlassen.
  // Stattdessen schließen und vom Knopf aus weitertabben lassen.
  const onPanelKeyDown = (event: React.KeyboardEvent) => {
    if (event.key !== "Tab" || !panelRef.current) return;
    if (hasOpenSelectMenu(panelRef.current)) return;
    const list = Array.from(
      panelRef.current.querySelectorAll<HTMLElement>(
        'input,button,select,textarea,[tabindex]:not([tabindex="-1"])',
      ),
    ).filter((el) => !(el as HTMLInputElement).disabled);
    const active = document.activeElement;
    const atEdge = event.shiftKey
      ? active === list[0]
      : active === list[list.length - 1];
    if (!atEdge && list.length > 0) return;
    event.preventDefault();
    setOpen(false);
    buttonRef.current?.focus();
  };

  return (
    <div
      ref={wrapRef}
      className="contents"
      onKeyDownCapture={(event) => {
        if (!open || event.key !== "Escape") return;
        // Ein offenes Auswahlmenü im Popover schließt zuerst sich selbst.
        if (hasOpenSelectMenu(panelRef.current)) return;
        event.preventDefault();
        event.stopPropagation();
        setOpen(false);
        buttonRef.current?.focus();
      }}
    >
      <IconAction
        ref={buttonRef}
        {...trigger}
        suppressTooltip={open}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? panelId : undefined}
        onClick={() => setOpen((o) => !o)}
      />
      {open &&
        createPortal(
          <div
            ref={panelRef}
            id={panelId}
            role="dialog"
            aria-label={popoverLabel}
            data-testid={popoverTestId}
            style={placementStyle}
            onKeyDown={onPanelKeyDown}
            className="z-40 space-y-2 rounded-lg border border-mid-gray/40 bg-background p-3 shadow-lg"
          >
            {children}
          </div>,
          document.body,
        )}
    </div>
  );
};
