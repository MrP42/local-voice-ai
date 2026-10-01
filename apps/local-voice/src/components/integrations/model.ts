import type { Caller, Direction, GrantMode, Kind } from "@/bindings";

/** Aufrufer-Spalten der Rechte-Matrix (Reihenfolge wie im Backend). */
export const MATRIX_CALLERS: Caller[] = [
  "workflow",
  "agent_external",
  "agent_local",
];

export const GRANT_MODES: GrantMode[] = ["off", "ask", "allow"];

export const DIRECTIONS: Direction[] = ["read", "write", "both"];

/** Fehler eines Kommandos als Text: ein Code (`folder_path_missing`) oder Klartext. */
export const errorText = (error: unknown): string => {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "";
};

/** Pfad aus der Konfiguration einer Ordner-Integration. */
export const folderPathOf = (configJson: string): string | null => {
  try {
    const parsed = JSON.parse(configJson) as { path?: unknown };
    return typeof parsed.path === "string" && parsed.path ? parsed.path : null;
  } catch {
    return null;
  }
};

/** Arten, die im Katalog stehen. `id` ist der Schluessel der Texte. */
export type CatalogStatus = "available" | "auto" | "soon";

export interface CatalogEntry {
  id: string;
  /** Art im Register (nur wo es eine gibt). */
  kind: Kind | null;
  status: CatalogStatus;
}

/**
 * Der Katalog. „available“: laesst sich hier einrichten. „auto“: legt sich
 * selbst an (YouTube mit dem ersten Link). „soon“: kommt mit einem der
 * naechsten Pakete (Konten und Ziele A5/A6, Agenten A7, Webhook Goal B).
 */
export const CATALOG: CatalogEntry[] = [
  { id: "calendar_ics", kind: "ics", status: "available" },
  { id: "calendar_graph", kind: "graph", status: "available" },
  { id: "folder", kind: "folder", status: "available" },
  { id: "youtube", kind: "youtube", status: "auto" },
  { id: "mcp", kind: "agent", status: "available" },
  { id: "m365", kind: "m365", status: "available" },
  { id: "smtp", kind: "smtp", status: "soon" },
  { id: "obsidian", kind: "obsidian", status: "soon" },
  { id: "wissen", kind: "wissen", status: "soon" },
  { id: "webhook", kind: null, status: "soon" },
];

export const capabilityKey = (capability: string): string =>
  `integrations.capabilities.${capability.replace(".", "_")}`;
