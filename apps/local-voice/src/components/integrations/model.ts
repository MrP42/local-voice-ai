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

/** Ein Feld der Konfiguration (ohne Geheimnisse) als Text; leer, wenn es fehlt. */
export const configText = (configJson: string, key: string): string => {
  try {
    const value = (JSON.parse(configJson) as Record<string, unknown>)[key];
    if (typeof value === "string") return value;
    if (typeof value === "number") return String(value);
    return "";
  } catch {
    return "";
  }
};

/** Arten mit eigenem Einstellungsformular (A6): Konto oder Ziel mit Pfad/Adresse. */
export type TargetKind = "folder" | "smtp" | "obsidian" | "wissen";

export const TARGET_KINDS: readonly Kind[] = [
  "folder",
  "smtp",
  "obsidian",
  "wissen",
];

export const isTargetKind = (kind: Kind): kind is TargetKind =>
  TARGET_KINDS.includes(kind);

/** Die zehn Kontextbereiche des AI-OS und die Stufen der Schreib-Autonomie. */
export const CONTEXT_AREAS = [
  "privat",
  "familie",
  "beruf",
  "wai",
  "schule_uni",
  "finanzen",
  "gesundheit",
  "behoerden",
  "projekte",
  "kunden",
] as const;
export const TIERS = ["auto", "logged", "propose", "untouchable"] as const;

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
 * naechsten Pakete (Microsoft 365 A5, Webhook Goal B).
 */
export const CATALOG: CatalogEntry[] = [
  { id: "calendar_ics", kind: "ics", status: "available" },
  { id: "calendar_graph", kind: "graph", status: "available" },
  { id: "folder", kind: "folder", status: "available" },
  { id: "youtube", kind: "youtube", status: "auto" },
  { id: "mcp", kind: "agent", status: "available" },
  { id: "m365", kind: "m365", status: "soon" },
  { id: "smtp", kind: "smtp", status: "available" },
  { id: "obsidian", kind: "obsidian", status: "available" },
  { id: "wissen", kind: "wissen", status: "available" },
  { id: "webhook", kind: null, status: "soon" },
];

export const capabilityKey = (capability: string): string =>
  `integrations.capabilities.${capability.replace(".", "_")}`;
