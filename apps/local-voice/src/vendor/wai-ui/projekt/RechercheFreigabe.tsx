import { useEffect, useId, useMemo, useRef, useState } from "react";
import { Dialog } from "../components/Dialog";

export interface FreigabeAnfrage {
  id: string;
  text: string;
  /** Warnhinweis zur Zeile („enthält möglicherweise Personenbezug“). */
  hint?: string;
  /** Anfangszustand; ohne Angabe gewählt, mit Hinweis abgewählt. */
  selected?: boolean;
}

export interface RechercheFreigabeLabels {
  title: string;
  context: (label: string) => string;
  externalNote: string;
  queries: string;
  send: string;
  projectOnly: string;
  cancel: string;
  none: string;
}

export const FREIGABE_LABELS: RechercheFreigabeLabels = {
  title: "Ausgewählte Suchanfragen an externe Suchmaschinen senden?",
  context: (l) => `Kontext: ${l}.`,
  externalNote: "Nur markierte Anfragen werden gesendet.",
  queries: "Suchanfragen",
  send: "Ausgewählte senden",
  projectOnly: "Nur im Projekt suchen",
  cancel: "Abbrechen",
  none: "Keine Anfrage ausgewählt: Es wird nichts gesendet.",
};

export interface RechercheFreigabeProps {
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  /** Kontext in Worten, z. B. Besprechung „Personalgespräche“ (personenbezogen). */
  contextLabel: string;
  queries: readonly FreigabeAnfrage[];
  onSend: (ids: readonly string[]) => void;
  /** Ohne diese Funktion gibt es keinen Knopf „Nur im Projekt suchen“. */
  onProjectOnly?: () => void;
  /** Entscheidung wird gerade verarbeitet: Knöpfe gesperrt. */
  busy?: boolean;
  /** Fehler der letzten Entscheidung; erscheint im Dialog, damit er nicht dahinter verschwindet. */
  error?: string;
  onCancel?: () => void;
  labels?: Partial<RechercheFreigabeLabels>;
}

const anfang = (qs: readonly FreigabeAnfrage[]) =>
  new Set(qs.filter((q) => q.selected ?? !q.hint).map((q) => q.id));

export function RechercheFreigabe({
  isOpen,
  onOpenChange,
  contextLabel,
  queries,
  onSend,
  onProjectOnly,
  onCancel,
  busy = false,
  error,
  labels: overrides,
}: RechercheFreigabeProps) {
  const L = useMemo(() => ({ ...FREIGABE_LABELS, ...overrides }), [overrides]);
  const ids = useId();
  const [gewaehlt, setGewaehlt] = useState(() => anfang(queries));
  const aktuell = useRef(queries);
  aktuell.current = queries;
  const warOffen = useRef(isOpen);
  useEffect(() => {
    // Nur beim Öffnen neu vorbelegen: ein neues Array je Render darf die Wahl nicht zurücksetzen.
    if (isOpen && !warOffen.current) setGewaehlt(anfang(aktuell.current));
    warOffen.current = isOpen;
  }, [isOpen]);

  const umschalten = (id: string, an: boolean) =>
    setGewaehlt((alt) => {
      const neu = new Set(alt);
      if (an) neu.add(id);
      else neu.delete(id);
      return neu;
    });

  return (
    <Dialog
      isOpen={isOpen}
      onOpenChange={(offen) => {
        if (!offen) onCancel?.();
        onOpenChange(offen);
      }}
      title={L.title}
      role="alertdialog"
      isDismissable={false}
      closeLabel={L.cancel}
      initialFocus="close"
      primaryAction={{
        label: L.send,
        isDisabled: gewaehlt.size === 0 || busy,
        onPress: () => onSend(queries.filter((q) => gewaehlt.has(q.id)).map((q) => q.id)),
      }}
      secondaryAction={
        onProjectOnly
          ? { label: L.projectOnly, onPress: onProjectOnly, isDisabled: busy }
          : undefined
      }
      breit
    >
      <div className="wai-freigabe">
        <p>{L.externalNote}</p>
        <p>{L.context(contextLabel)}</p>
        <ul className="wai-freigabe__liste" aria-label={L.queries}>
          {queries.map((q) => (
            <li key={q.id}>
              <label>
                <input
                  type="checkbox"
                  checked={gewaehlt.has(q.id)}
                  aria-describedby={q.hint ? `${ids}-${q.id}` : undefined}
                  onChange={(e) => umschalten(q.id, e.target.checked)}
                />{" "}
                „{q.text}“
              </label>
              {q.hint && (
                <small id={`${ids}-${q.id}`} className="wai-freigabe__hinweis">
                  {q.hint}
                </small>
              )}
            </li>
          ))}
        </ul>
        {gewaehlt.size === 0 && <p className="wai-freigabe__hinweis">{L.none}</p>}
        {error && <p role="alert">{error}</p>}
      </div>
    </Dialog>
  );
}
