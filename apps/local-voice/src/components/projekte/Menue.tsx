import React, {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

export type MenueZeile =
  | { kopf: string }
  | {
      id: string;
      label: string;
      /** Leise Angabe rechts (Token, „Vorgabe“). */
      rechts?: string;
      /** Gesetzt: Auswahlpunkt (`menuitemradio`), sonst normaler Punkt. */
      gewaehlt?: boolean;
      gefahr?: boolean;
    };

interface Props {
  /** Name des Knopfes für Hilfstechnik. */
  label: string;
  /** Name des geöffneten Menüs. */
  menueLabel: string;
  knopf: React.ReactNode;
  className?: string;
  zeilen: MenueZeile[];
  onWahl: (id: string) => void;
  align?: "start" | "end";
  testId?: string;
}

const RAND = 8;

/**
 * Knopf mit Menü (Form wählen, Weitere Aktionen) für die Projektseite. Das Menü hängt per Portal
 * an `body` und ist fest platziert: die scrollende Liste schneidet es nicht ab. Ziele sind mindestens
 * 44 px hoch; Pfeiltasten wandern, Esc schließt und gibt den Fokus an den Knopf zurück.
 */
export const Menue: React.FC<Props> = ({
  label,
  menueLabel,
  knopf,
  className = "",
  zeilen,
  onWahl,
  align = "end",
  testId,
}) => {
  const [offen, setOffen] = useState(false);
  const [platz, setPlatz] = useState<{
    top: number;
    left: number;
    maxHeight: number;
  } | null>(null);
  const knopfRef = useRef<HTMLButtonElement>(null);
  const menueRef = useRef<HTMLDivElement>(null);
  const id = useId();

  const zu = useCallback((fokus: boolean) => {
    setOffen(false);
    if (fokus) knopfRef.current?.focus();
  }, []);

  useLayoutEffect(() => {
    if (!offen) {
      setPlatz(null);
      return;
    }
    const rechne = () => {
      const k = knopfRef.current?.getBoundingClientRect();
      const m = menueRef.current;
      if (!k || !m) return;
      const breite = m.offsetWidth;
      const hoehe = m.scrollHeight;
      const vw = document.documentElement.clientWidth;
      const vh = window.innerHeight;
      const links = align === "end" ? k.right - breite : k.left;
      const unten = vh - k.bottom - RAND;
      const oben = k.top - RAND;
      const nachOben = hoehe > unten && oben > unten;
      const max = Math.max(120, nachOben ? oben : unten);
      setPlatz({
        left: Math.min(
          Math.max(RAND, links),
          Math.max(RAND, vw - breite - RAND),
        ),
        top: nachOben
          ? Math.max(RAND, k.top - Math.min(hoehe, max) - 4)
          : k.bottom + 4,
        maxHeight: max,
      });
    };
    rechne();
    window.addEventListener("resize", rechne);
    window.addEventListener("scroll", rechne, true);
    return () => {
      window.removeEventListener("resize", rechne);
      window.removeEventListener("scroll", rechne, true);
    };
  }, [offen, align]);

  const platziert = platz !== null;
  // Erst fokussieren, wenn das Menü sichtbar platziert ist (unsichtbar nimmt es keinen Fokus).
  useEffect(() => {
    if (!offen || !platziert) return;
    const punkte =
      menueRef.current?.querySelectorAll<HTMLElement>('[role^="menuitem"]');
    const gewaehlt = menueRef.current?.querySelector<HTMLElement>(
      '[aria-checked="true"]',
    );
    (gewaehlt ?? punkte?.[0])?.focus();
  }, [offen, platziert]);

  useEffect(() => {
    if (!offen) return;
    const aussen = (e: MouseEvent) => {
      const ziel = e.target as Node;
      if (menueRef.current?.contains(ziel) || knopfRef.current?.contains(ziel))
        return;
      zu(false);
    };
    document.addEventListener("mousedown", aussen);
    return () => document.removeEventListener("mousedown", aussen);
  }, [offen, zu]);

  const taste = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      zu(true);
      return;
    }
    if (e.key === "Tab") {
      zu(false);
      return;
    }
    const punkte = Array.from(
      menueRef.current?.querySelectorAll<HTMLElement>('[role^="menuitem"]') ??
        [],
    );
    const i = punkte.indexOf(document.activeElement as HTMLElement);
    const geh = (n: number) => {
      e.preventDefault();
      punkte[(n + punkte.length) % punkte.length]?.focus();
    };
    if (e.key === "ArrowDown") geh(i + 1);
    else if (e.key === "ArrowUp") geh(i - 1);
    else if (e.key === "Home") geh(0);
    else if (e.key === "End") geh(punkte.length - 1);
  };

  return (
    <>
      <button
        ref={knopfRef}
        type="button"
        className={className}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={offen}
        aria-controls={offen ? id : undefined}
        data-testid={testId}
        onClick={() => setOffen((o) => !o)}
      >
        {knopf}
      </button>
      {offen &&
        createPortal(
          <div
            ref={menueRef}
            id={id}
            role="menu"
            aria-label={menueLabel}
            className="pb-menue"
            style={{
              top: platz?.top ?? 0,
              left: platz?.left ?? 0,
              maxHeight: platz?.maxHeight,
              visibility: platz ? "visible" : "hidden",
            }}
            onKeyDown={taste}
          >
            {zeilen.map((z, i) =>
              "kopf" in z ? (
                <div
                  key={`k${i}`}
                  className="pb-menue__kopf"
                  role="presentation"
                >
                  {z.kopf}
                </div>
              ) : (
                <button
                  key={z.id}
                  type="button"
                  role={z.gewaehlt === undefined ? "menuitem" : "menuitemradio"}
                  aria-checked={
                    z.gewaehlt === undefined ? undefined : z.gewaehlt
                  }
                  className={`pb-menue__punkt${z.gewaehlt ? " pb-menue__punkt--an" : ""}${z.gefahr ? " pb-menue__punkt--gefahr" : ""}`}
                  data-testid={`menue-${z.id}`}
                  onClick={() => {
                    zu(true);
                    onWahl(z.id);
                  }}
                >
                  <span>{z.label}</span>
                  {z.rechts && (
                    <span className="pb-menue__rechts">{z.rechts}</span>
                  )}
                </button>
              ),
            )}
          </div>,
          document.body,
        )}
    </>
  );
};
