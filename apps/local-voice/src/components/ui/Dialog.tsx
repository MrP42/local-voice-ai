import React, { createContext, useContext, useEffect, useRef, useState } from "react";
import {
  Button as FluentButton,
  Dialog as FluentDialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Tooltip as FluentTooltip,
} from "@fluentui/react-components";
import { X } from "lucide-react";

/** Tailwind-Breiten der Aufrufer (`max-w-2xl` …) in rem. Fluent setzt die
 *  Breite der Dialogfläche selbst (ungeschichtet), Tailwind-Klassen kämen
 *  dagegen nicht an — deshalb als Inline-Stil. */
const MAX_WIDTH_REM: Record<string, number> = {
  sm: 24, md: 28, lg: 32, xl: 36, "2xl": 42, "3xl": 48, "4xl": 56, "5xl": 64, "6xl": 72, "7xl": 80,
};
const maxWidthOf = (className: string): string | undefined => {
  const m = className.match(/(?:^|\s)max-w-(\w+|\[[^\]]+\])(?:\s|$)/);
  if (!m) return undefined;
  if (m[1].startsWith("[")) return m[1].slice(1, -1);
  const rem = MAX_WIDTH_REM[m[1]];
  return rem ? `${rem}rem` : undefined;
};

/** Dialogfläche als Ziel für schwebende Listen (react-select `menuPortal`):
 *  Ein modaler Fluent-Dialog blendet alles außerhalb für Hilfstechnik aus
 *  (aria-hidden); eine Liste an `document.body` wäre dann unerreichbar. */
const DialogPortalContext = createContext<HTMLElement | null>(null);
export const useDialogPortalTarget = () => useContext(DialogPortalContext);

interface DialogProps {
  open: boolean;
  title: React.ReactNode;
  children: React.ReactNode;
  onOpenChange: (open: boolean) => void;
  description?: React.ReactNode;
  footer?: React.ReactNode;
  closeLabel: string;
  dismissible?: boolean;
  closeOnBackdrop?: boolean;
  showCloseButton?: boolean;
  initialFocusRef?: React.RefObject<HTMLElement>;
  className?: string;
  contentClassName?: string;
  contentFades?: boolean;
}

/**
 * Dialog nach Fluent 2 (Leitlinie windows-fluent.md §5): Fluent Dialog mit
 * Titel als Frage, Fußleiste mit Primär links und Abbrechen rechts (die
 * Reihenfolge stellt fluent-bridge.css her), Esc schließt, Fokus bleibt im
 * Dialog und kehrt zum Auslöser zurück.
 *
 * Das Verhalten der bisherigen Komponente bleibt: `dismissible={false}`
 * ignoriert Esc und Hintergrund, `closeOnBackdrop={false}` nur den Hintergrund,
 * `initialFocusRef` bekommt den ersten Fokus, sonst die Dialogfläche.
 */
export const Dialog: React.FC<DialogProps> = ({
  open,
  title,
  children,
  onOpenChange,
  description,
  footer,
  closeLabel,
  dismissible = true,
  closeOnBackdrop = true,
  showCloseButton = true,
  initialFocusRef,
  className = "",
  contentClassName = "",
  contentFades = true,
}) => {
  const surfaceRef = useRef<HTMLDivElement>(null);
  const [surface, setSurface] = useState<HTMLElement | null>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const descriptionId = React.useId();

  useEffect(() => {
    if (!open) return;
    previousFocusRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const frame = requestAnimationFrame(() => {
      const target = initialFocusRef?.current ?? surfaceRef.current;
      target?.focus();
    });
    return () => {
      cancelAnimationFrame(frame);
      const previous = previousFocusRef.current;
      previousFocusRef.current = null;
      if (previous?.isConnected) previous.focus();
    };
  }, [initialFocusRef, open]);

  const contentStyle: React.CSSProperties | undefined = contentFades
    ? {
        maskImage:
          "linear-gradient(to bottom, transparent 0, black 10px, black calc(100% - 20px), transparent 100%)",
        WebkitMaskImage:
          "linear-gradient(to bottom, transparent 0, black 10px, black calc(100% - 20px), transparent 100%)",
      }
    : undefined;

  const hasChildren =
    children !== null && children !== undefined && children !== false;

  return (
    <FluentDialog
      open={open}
      modalType={dismissible ? "modal" : "alert"}
      // Ohne Ein-/Ausblend-Bewegung: der Dialog ist sofort da und sofort weg
      // (Fokus und Folgeansicht ohne Zwischenzustand, wie bisher).
      surfaceMotion={null}
      onOpenChange={(_event, data) => {
        if (data.open) return;
        if (!dismissible) return;
        if (data.type === "backdropClick" && !closeOnBackdrop) return;
        onOpenChange(false);
      }}
    >
      <DialogSurface
        ref={(el: HTMLDivElement | null) => {
          (surfaceRef as React.MutableRefObject<HTMLDivElement | null>).current = el;
          setSurface(el);
        }}
        tabIndex={-1}
        aria-describedby={description ? descriptionId : undefined}
        className={`lv-dialog ${className}`}
        style={{ maxWidth: maxWidthOf(className) }}
      >
        <DialogPortalContext.Provider value={surface}>
        <DialogBody className="lv-dialog__body">
          <DialogTitle
            action={
              dismissible && showCloseButton ? (
                <FluentTooltip content={closeLabel} relationship="label">
                  <FluentButton
                    appearance="subtle"
                    size="small"
                    icon={<X className="h-4 w-4" aria-hidden="true" />}
                    onClick={() => onOpenChange(false)}
                  />
                </FluentTooltip>
              ) : undefined
            }
          >
            {title}
          </DialogTitle>
          {(description || hasChildren) && (
            <DialogContent
              className={`lv-dialog__content ${contentClassName}`}
              style={contentStyle}
            >
              {description && (
                <p
                  id={descriptionId}
                  className="lv-dialog__description text-sm text-text-muted"
                >
                  {description}
                </p>
              )}
              {hasChildren && children}
            </DialogContent>
          )}
          {footer && (
            <DialogActions
              data-testid="dialog-footer"
              fluid
              className="lv-dialog__actions"
            >
              {footer}
            </DialogActions>
          )}
        </DialogBody>
        </DialogPortalContext.Provider>
      </DialogSurface>
    </FluentDialog>
  );
};
