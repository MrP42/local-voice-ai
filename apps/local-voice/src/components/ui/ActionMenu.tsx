import React, { useEffect, useId, useRef, useState } from "react";
import type { LucideIcon } from "lucide-react";
import { IconAction, type IconActionProps } from "./IconAction";

// Menü und Popover an einem Symbol-Knopf. Beide hängen bewusst im normalen
// Dokumentfluss (absolut positioniert, kein Portal): so folgt die Tab-Reihenfolge
// dem Knopf, und die Bedienspalte muss keine Koordinaten nachrechnen.

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
 *  anderen Knopf nicht erst nach dem Schließen ankommt). */
const useOutsideClose = (
  ref: React.RefObject<HTMLElement>,
  open: boolean,
  onClose: () => void,
) => {
  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) {
        onClose();
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open, onClose, ref]);
};

const MENU_CLASSES =
  "absolute top-full z-40 mt-1 rounded-lg border border-mid-gray/40 bg-background py-1 shadow-lg";

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
  useOutsideClose(wrapRef, open, closeOutside);

  // Beim Öffnen landet der Fokus im Menü: erster Eintrag (mit Pfeil nach oben
  // vom Knopf aus: letzter).
  const focusTarget = useRef<"first" | "last">("first");
  useEffect(() => {
    if (!open) return;
    const list = focusables();
    (focusTarget.current === "last" ? list[list.length - 1] : list[0])?.focus();
    focusTarget.current = "first";
  }, [open]);

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
      {open && (
        <div
          ref={menuRef}
          id={menuId}
          role="menu"
          aria-label={menuLabel}
          className={`${MENU_CLASSES} ${widthClass} ${
            align === "end" ? "end-0" : "start-0"
          }`}
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
        </div>
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
 * (z. B. Umfang, Detailgrad, Zielgruppe). Füllt die Breite der nächsten
 * positionierten Zeile, damit es nicht über die Spalte hinausragt.
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
  const closeOutside = React.useCallback(() => setOpen(false), []);
  useOutsideClose(wrapRef, open, closeOutside);

  useEffect(() => {
    if (!open) return;
    panelRef.current
      ?.querySelector<HTMLElement>("input,button,select,textarea,[tabindex]")
      ?.focus();
  }, [open]);

  return (
    <div
      ref={wrapRef}
      className="contents"
      onKeyDownCapture={(event) => {
        if (!open || event.key !== "Escape") return;
        // Ein offenes Auswahlmenü im Popover schließt zuerst sich selbst.
        if (panelRef.current?.querySelector(".app-select__menu")) return;
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
      {open && (
        <div
          ref={panelRef}
          id={panelId}
          role="dialog"
          aria-label={popoverLabel}
          data-testid={popoverTestId}
          className="absolute inset-x-0 top-full z-40 mt-1 space-y-2 rounded-lg border border-mid-gray/40 bg-background p-3 shadow-lg"
        >
          {children}
        </div>
      )}
    </div>
  );
};
