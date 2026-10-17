/**
 * @wai/ui erscheinungsbild v1 — Fensterknoepfe.tsx
 * Quelle: WAI-SDK packages/ui/src/erscheinungsbild (Vorbild: Poststelle, komponenten/Fensterknoepfe.tsx).
 * Selbsttragend: importiert nur react und @fluentui/react-components; keine Tauri-Abhaengigkeit.
 *
 * Fensterknoepfe wie Windows 11 (Minimieren, Maximieren/Wiederherstellen, Schliessen) fuer rahmenlose
 * Fenster. Die App reicht die Handler hinein, z. B. fuer Tauri:
 *   const w = getCurrentWindow();
 *   <Fensterknoepfe minimieren={() => w.minimize()} maximierenUmschalten={() => w.toggleMaximize()}
 *     schliessen={() => w.close()} istMaximiert={() => w.isMaximized()}
 *     beiGroessenaenderung={(cb) => w.onResized(cb)} />
 */
import { makeStyles, mergeClasses, tokens } from "@fluentui/react-components";
import { useEffect, useState } from "react";

type Vielleicht<T> = T | Promise<T>;

export interface FensterHandler {
  minimieren: () => Vielleicht<void>;
  maximierenUmschalten: () => Vielleicht<void>;
  schliessen: () => Vielleicht<void>;
  /** Liest den Fensterzustand (beim Start und nach jeder Groessenaenderung). */
  istMaximiert?: () => Vielleicht<boolean>;
  /** Meldet Groessenaenderungen; gibt die Abmeldung zurueck. */
  beiGroessenaenderung?: (rueckruf: () => void) => Vielleicht<() => void>;
}

export interface FensterknoepfeTexte {
  gruppe: string;
  minimieren: string;
  maximieren: string;
  wiederherstellen: string;
  schliessen: string;
}

export const FENSTER_TEXTE: FensterknoepfeTexte = {
  gruppe: "Fenster",
  minimieren: "Minimieren",
  maximieren: "Maximieren",
  wiederherstellen: "Wiederherstellen",
  schliessen: "Schließen",
};

export interface FensterknoepfeProps extends FensterHandler {
  texte?: Partial<FensterknoepfeTexte>;
  /** Hoehe der Knoepfe in px (Windows 11: 48 bei hoher Titelleiste, 32 bei kompakter). */
  hoehe?: number;
  className?: string;
}

const useStyles = makeStyles({
  gruppe: { display: "flex", alignSelf: "stretch", flex: "none" },
  knopf: {
    display: "grid",
    placeItems: "center",
    width: "46px",
    padding: 0,
    border: "none",
    borderRadius: 0,
    backgroundColor: "transparent",
    color: tokens.colorNeutralForeground1,
    cursor: "default",
    outlineStyle: "none",
    // Fensterknoepfe sind keine Ziehflaeche (Tauri/Electron: data-tauri-drag-region auf der Leiste).
    WebkitAppRegion: "no-drag",
    ":hover": { backgroundColor: tokens.colorSubtleBackgroundHover },
    ":active": { backgroundColor: tokens.colorSubtleBackgroundPressed },
    ":focus-visible": {
      outline: `${tokens.strokeWidthThick} solid ${tokens.colorStrokeFocus2}`,
      outlineOffset: "-2px",
    },
  },
  schliessen: {
    ":hover": { backgroundColor: "#c42b1c", color: "#ffffff" },
    ":active": { backgroundColor: "#c42b1ce6", color: "#ffffff" },
  },
});

/** Glyphen nach Segoe-Fluent-Vorbild: 10 px, 1 px Linie. */
function Glyphe({ art }: { art: "min" | "max" | "wieder" | "zu" }) {
  return (
    <svg
      width="10"
      height="10"
      viewBox="0 0 10 10"
      aria-hidden="true"
      focusable="false"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
    >
      {art === "min" ? <path d="M0 5.5h10" /> : null}
      {art === "max" ? <rect x="0.5" y="0.5" width="9" height="9" rx="1" /> : null}
      {art === "wieder" ? (
        <>
          <rect x="0.5" y="2.5" width="7" height="7" rx="1" />
          <path d="M2.5 2.5V1.5a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-1" />
        </>
      ) : null}
      {art === "zu" ? <path d="M0.5 0.5l9 9M9.5 0.5l-9 9" /> : null}
    </svg>
  );
}

const leise = (f: () => Vielleicht<void>) => () => {
  try {
    void Promise.resolve(f()).catch(() => undefined);
  } catch {
    /* Fenster nicht erreichbar: Knopf bleibt ohne Wirkung */
  }
};

export function Fensterknoepfe({
  minimieren,
  maximierenUmschalten,
  schliessen,
  istMaximiert,
  beiGroessenaenderung,
  texte,
  hoehe = 48,
  className,
}: FensterknoepfeProps) {
  const s = useStyles();
  const t = { ...FENSTER_TEXTE, ...texte };
  const [maximiert, setMaximiert] = useState(false);

  useEffect(() => {
    if (!istMaximiert) return;
    let vorbei = false;
    let ab: (() => void) | undefined;
    const pruefe = () =>
      void Promise.resolve()
        .then(istMaximiert)
        .then((m) => !vorbei && setMaximiert(m))
        .catch(() => undefined);
    pruefe();
    if (beiGroessenaenderung) {
      Promise.resolve()
        .then(() => beiGroessenaenderung(pruefe))
        .then((weg) => {
          if (vorbei) weg();
          else ab = weg;
        })
        .catch(() => undefined);
    }
    return () => {
      vorbei = true;
      ab?.();
    };
  }, [istMaximiert, beiGroessenaenderung]);

  const maxText = maximiert ? t.wiederherstellen : t.maximieren;
  return (
    // Bewusst ohne Tooltip-Komponente: Windows zeigt an den Fensterknoepfen nur den nativen Hinweis.
    // biome-ignore lint/a11y/useSemanticElements: Knopfgruppe in der Titelleiste, kein Formular (fieldset unpassend)
    <div className={mergeClasses(s.gruppe, className)} role="group" aria-label={t.gruppe}>
      <button
        type="button"
        className={s.knopf}
        style={{ height: hoehe }}
        aria-label={t.minimieren}
        title={t.minimieren}
        onClick={leise(minimieren)}
      >
        <Glyphe art="min" />
      </button>
      <button
        type="button"
        className={s.knopf}
        style={{ height: hoehe }}
        aria-label={maxText}
        title={maxText}
        onClick={leise(maximierenUmschalten)}
      >
        <Glyphe art={maximiert ? "wieder" : "max"} />
      </button>
      <button
        type="button"
        className={mergeClasses(s.knopf, s.schliessen)}
        style={{ height: hoehe }}
        aria-label={t.schliessen}
        title={t.schliessen}
        onClick={leise(schliessen)}
      >
        <Glyphe art="zu" />
      </button>
    </div>
  );
}
