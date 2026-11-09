import {
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Dialog as FluentDialog,
} from "@fluentui/react-components";
import { useActivateModal } from "@fluentui/react-tabster";
import {
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { Button } from "./Button";

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select, textarea, [tabindex]:not([tabindex="-1"])';

/**
 * Die Fokus- und Freigabe-Timer laufen bewusst nach dem Aushaengen weiter. Ist das DOM dann fort
 * (Server, abgebaute Testumgebung, #184), tun sie nichts.
 */
const ohneDom = () => typeof document === "undefined";
/** Verzoegerte zweite Freigabe; je Zeitpunkt nur eine, ein neuer Aufruf ersetzt die alte. */
let nachlauf: ReturnType<typeof setTimeout> | undefined;

/**
 * Ohne offenes Modal darf auf oberster Ebene nichts fuer Screenreader verborgen sein. tabster raeumt
 * aria-hidden nicht auf, wenn ein Modal waehrend der Ausblend-Animation ausgehaengt wird.
 */
function releaseBackground() {
  if (ohneDom() || document.querySelector('[aria-modal="true"]')) return;
  for (const el of Array.from(document.body.children)) {
    if (el.getAttribute("aria-hidden") === "true" && !el.hasAttribute("data-tabster-dummy")) {
      el.removeAttribute("aria-hidden");
    }
  }
}

/** Fokus nach dem Schliessen: Ausloeser, sonst Hauptbereich, sonst erstes bedienbares Element. */
/** Umgebung des Ausloesers, falls dieser nach dem Schliessen fehlt (z. B. geloeschte Zeile). */
const SCOPE = '[role="tabpanel"], main, [role="main"]';

function refocus(opener: HTMLElement | null, scope: HTMLElement | null = null) {
  if (ohneDom()) return;
  releaseBackground();
  clearTimeout(nachlauf);
  nachlauf = setTimeout(releaseBackground, 300); // nach tabsters verzoegerter Aktualisierung (250 ms) erneut
  if (opener?.isConnected) return opener.focus();
  const target = scope?.isConnected
    ? scope
    : document.querySelector<HTMLElement>("main, [role=main]");
  if (target) {
    if (!target.hasAttribute("tabindex")) target.setAttribute("tabindex", "-1");
    return target.focus({ preventScroll: true });
  }
  // Nie auf einen Tab: bei automatischer Aktivierung wuerde der Fokus die Ansicht wechseln.
  const first = Array.from(document.body.querySelectorAll<HTMLElement>(FOCUSABLE)).find(
    (e) =>
      !e.closest('[role="dialog"], [role="alertdialog"], [data-tabster-dummy]') &&
      e.getAttribute("role") !== "tab",
  );
  first?.focus();
}

/**
 * Fokusrückgabe für modale Überlagerungen (Dialog, Drawer): merkt den Auslöser beim Öffnen, gibt den
 * Fokus beim Schließen oder Aushängen zurück und räumt aria-hidden am Hintergrund auf.
 */
export function useFocusReturn(isOpen: boolean) {
  const opener = useRef<HTMLElement | null>(null);
  const scope = useRef<HTMLElement | null>(null);
  const activateModal = useActivateModal();
  // Zeitpunkt des letzten Schliessens: Fluent spielt danach noch die Ausblend-Animation.
  const closedAt = useRef(0);
  // Layout-Effekt: laeuft, bevor Fluent den Fokus in den Dialog setzt.
  useLayoutEffect(() => {
    if (isOpen) {
      opener.current = document.activeElement as HTMLElement | null;
      scope.current = opener.current?.closest<HTMLElement>(SCOPE) ?? null;
      return;
    }
    const el = opener.current;
    if (!el) return; // war nie offen: Fokus nicht anfassen
    opener.current = null;
    closedAt.current = Date.now();
    const sc = scope.current;
    setTimeout(() => refocus(el, sc), 0);
  }, [isOpen]);
  // Wird der Dialog offen ausgehaengt (z. B. Objekt geloescht, Seite gewechselt), gibt Fluent den
  // Hintergrund erst beim naechsten Fokuswechsel frei (aria-hidden). Fokus daher sinnvoll setzen.
  useLayoutEffect(
    () => () => {
      // Offen ODER noch in der Ausblend-Animation ausgehaengt (Objekt geloescht, Seite gewechselt).
      const closing = Date.now() - closedAt.current < 1000;
      if (!opener.current && !closing) return;
      const el = opener.current;
      opener.current = null;
      // tabster haelt das entfernte Modal sonst fuer aktiv und laesst aria-hidden stehen.
      activateModal(undefined);
      const sc = scope.current;
      setTimeout(() => refocus(el, sc), 0);
    },
    [activateModal],
  );
}

export interface DialogAction {
  label: string;
  onPress: () => void;
  isDisabled?: boolean;
  isPending?: boolean;
  /** Destruktive Hauptaktion (Loeschen) rot statt Akzent. */
  isDestructive?: boolean;
}

export interface DialogProps {
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  /** Frage oder Aussage, keine Floskel („Entwurf löschen?“ statt „Achtung“). */
  title: string;
  children: ReactNode;
  /** Sichere Aktion ganz rechts, reagiert auf Esc (Windows ContentDialog). */
  closeLabel?: string;
  /** Hauptaktion links, Akzent; beantwortet den Titel („Löschen“, nicht „OK“). */
  primaryAction?: DialogAction;
  secondaryAction?: DialogAction;
  /** Bei false schliesst Klick ausserhalb nicht (Pflichtentscheidung). */
  isDismissable?: boolean;
  role?: "dialog" | "alertdialog";
  /** Erstes fokussiertes Element: Hauptaktion (Standard) oder die sichere Schaltfläche „Schließen“. */
  initialFocus?: "primary" | "close";
  /** Breitere Fläche für Dialoge mit langen Zeilen und mehreren Knöpfen. */
  breit?: boolean;
}

/** Modaler Dialog auf Fluent 2: Fokusfalle, Escape und Titelbindung; Fokus kehrt zum Ausloeser zurueck. */
export function Dialog({
  isOpen,
  onOpenChange,
  title,
  children,
  closeLabel = "Schließen",
  primaryAction,
  secondaryAction,
  isDismissable = true,
  role = "dialog",
  initialFocus = "primary",
  breit = false,
}: DialogProps) {
  useFocusReturn(isOpen);
  const descId = useId();
  // Fokus-Absicherung: Schliesst ein Menue (Menueband, „…“) nach dem Oeffnen und gibt den Fokus an
  // seinen Ausloeser zurueck, liegt er ausserhalb des Modals; tabster blendet dann den Dialog selbst
  // aus (aria-hidden nach ca. 300 ms). Dann Fokus zurueck auf das erste Element bzw. die Flaeche.
  const surface = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!isOpen) return;
    const sichern = () => {
      const s = surface.current;
      if (!s?.isConnected || s.contains(document.activeElement)) return;
      const first = s.querySelector<HTMLElement>(FOCUSABLE);
      if (first) return first.focus();
      if (!s.hasAttribute("tabindex")) s.setAttribute("tabindex", "-1");
      s.focus();
    };
    const timers = [0, 100, 350].map((ms) => setTimeout(sichern, ms));
    return () => {
      for (const t of timers) clearTimeout(t);
    };
  }, [isOpen]);
  // Fluent fokussiert das erste bedienbare Element, sobald die Fläche gemountet ist. Dieser Effekt läuft
  // erst im Folge-Render (Zustand aus dem Ref-Callback) und damit nach Fluents Effekt: nur wenn der Fokus
  // noch dort (oder nirgends) liegt, wandert er auf die sichere Schaltfläche; Fokus des Nutzers bleibt.
  const closeButton = useRef<HTMLButtonElement>(null);
  const [mounted, setMounted] = useState(false);
  const surfaceRef = useCallback((el: HTMLDivElement | null) => {
    surface.current = el;
    setMounted(el !== null);
  }, []);
  useEffect(() => {
    if (!isOpen || initialFocus !== "close" || !mounted) return;
    const s = surface.current;
    const active = document.activeElement;
    if (active === document.body || active === s || active === s?.querySelector(FOCUSABLE))
      closeButton.current?.focus();
  }, [isOpen, initialFocus, mounted]);
  const close = () => onOpenChange(false);
  return (
    <FluentDialog
      open={isOpen}
      modalType={role === "alertdialog" ? "alert" : "modal"}
      onOpenChange={(_e, d) => {
        // Pflichtentscheidung: Klick daneben schliesst nicht, Escape und „Schließen“ schon.
        if (!isDismissable && d.type === "backdropClick") return;
        onOpenChange(d.open);
      }}
    >
      <DialogSurface
        ref={surfaceRef}
        className={breit ? "wai-modal wai-modal--breit" : "wai-modal"}
        aria-describedby={descId}
      >
        <DialogBody className="wai-dialog">
          <DialogTitle className="wai-dialog__title">{title}</DialogTitle>
          <DialogContent id={descId} className="wai-dialog__body">
            {children}
          </DialogContent>
          <DialogActions className="wai-dialog__actions">
            {primaryAction && (
              <Button
                variant={primaryAction.isDestructive ? "danger" : "primary"}
                isDisabled={primaryAction.isDisabled}
                isPending={primaryAction.isPending}
                onPress={primaryAction.onPress}
              >
                {primaryAction.label}
              </Button>
            )}
            {secondaryAction && (
              <Button
                isDisabled={secondaryAction.isDisabled}
                isPending={secondaryAction.isPending}
                onPress={secondaryAction.onPress}
              >
                {secondaryAction.label}
              </Button>
            )}
            <Button ref={closeButton} onPress={close}>
              {closeLabel}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </FluentDialog>
  );
}
