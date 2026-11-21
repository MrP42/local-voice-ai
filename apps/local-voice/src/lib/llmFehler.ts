/**
 * Fehlerformen des Backends für Sprachmodell-Verbindungen lesen (lva#654).
 * Das Backend liefert stabile Codes, die Texte stehen in den Übersetzungen.
 * Ohne React und ohne i18n, damit es ohne DOM testbar ist.
 */

/** Sperrgrund vor dem Installer: `cli_blocked:<code>`. */
export type CliSperre = "blocked" | "proxyAuth" | "proxy";

const CLI_SPERREN: ReadonlySet<string> = new Set([
  "blocked",
  "proxyAuth",
  "proxy",
]);

/** `cli_blocked:blocked` → `"blocked"`; alles andere `null`. */
export function cliSperre(roh: string): CliSperre | null {
  const code = /^cli_blocked:(\w+)/.exec(roh.trim())?.[1];
  return code && CLI_SPERREN.has(code) ? (code as CliSperre) : null;
}

/** Fehler beim Laden der Modellliste: `http_status:<code>[: <Fehlerkennung des Anbieters>]`. */
export interface ModellListenFehler {
  status: number;
  /** Fehlerkennung des Anbieters (nur `[a-z_]`, nie dessen Freitext). */
  anbieter: string;
}

export function modellListenFehler(roh: string): ModellListenFehler | null {
  const m = /^http_status:(\d{3})(?::\s*([\s\S]*))?$/.exec(roh.trim());
  if (!m) return null;
  return { status: Number(m[1]), anbieter: (m[2] ?? "").trim() };
}

/** Übersetzungsschlüssel (unter `settings.llm.connections.http`) je Status. */
export function modellListenSchluessel(status: number): string {
  switch (status) {
    case 401:
      return "unauthorized";
    case 403:
      return "forbidden";
    case 404:
      return "notFound";
    case 407:
      return "proxyAuth";
    case 429:
      return "rateLimited";
    default:
      return status >= 500 ? "server" : "other";
  }
}

/** Ursache eines gescheiterten Modell-Downloads: `net_fault:<code>`. */
export function netzFehlerCode(roh: string): string | null {
  return /^net_fault:(\w+)/.exec(roh.trim())?.[1] ?? null;
}
