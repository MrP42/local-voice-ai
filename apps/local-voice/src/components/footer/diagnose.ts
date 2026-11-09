import { useCallback, useEffect, useState } from "react";

// Statusleiste (ADR-0027 E7, Codex 01.11.): Standard sind nur Telefon und
// Mikrofon. Speicher/GPU, Fish-Server und das Prüfschild sind eine bewusste
// Diagnose-Option (Profilmenü ▸ Hilfe ▸ Diagnose) und dann einklappbar.
//   "aus" – nicht gezeigt (Standard)
//   "zu"  – eingeschaltet, eingeklappt (nur der Diagnose-Knopf)
//   "auf" – eingeschaltet, ausgeklappt
export type DiagnoseZustand = "aus" | "zu" | "auf";

const SCHLUESSEL = "lva.ui.statusbar.diagnose";
const EREIGNIS = "lva-diagnose-geaendert";

function lesen(): DiagnoseZustand {
  try {
    const wert = localStorage.getItem(SCHLUESSEL);
    return wert === "zu" || wert === "auf" ? wert : "aus";
  } catch {
    return "aus";
  }
}

export function useDiagnose(): [
  DiagnoseZustand,
  (zustand: DiagnoseZustand) => void,
] {
  const [zustand, setZustandLokal] = useState<DiagnoseZustand>(lesen);
  useEffect(() => {
    const aktualisieren = () => setZustandLokal(lesen());
    window.addEventListener(EREIGNIS, aktualisieren);
    window.addEventListener("storage", aktualisieren);
    return () => {
      window.removeEventListener(EREIGNIS, aktualisieren);
      window.removeEventListener("storage", aktualisieren);
    };
  }, []);
  const setZustand = useCallback((neu: DiagnoseZustand) => {
    try {
      localStorage.setItem(SCHLUESSEL, neu);
    } catch {
      // Ohne Speicher gilt die Wahl nur für diese Sitzung.
    }
    setZustandLokal(neu);
    window.dispatchEvent(new Event(EREIGNIS));
  }, []);
  return [zustand, setZustand];
}
