import type { TtsError, TtsProviderEntry } from "@/bindings";

/**
 * Reine Logik der Gruppe "Sprachausgabe-Anbieter" (ohne Oberflaeche, ohne
 * Tauri): Freigabe-Schritte, Konfigurationsfelder und Fehlertexte. Die
 * Oberflaeche liest nur Faehigkeiten und Zustaende des Anbieters -- es gibt
 * keine Zweige auf eine bestimmte Anbieter-Kennung.
 */

/** Der naechste Schritt der Freigabe, den der Nutzer noch tun muss. */
export type ApprovalStep = "license" | "remote" | null;

/**
 * Welcher Dialog als Naechstes faellig ist. Reihenfolge wie im Ablauf:
 * Lizenz bestaetigen, danach Fernzugriff freigeben. Ohne Opt-in gibt es
 * keinen Schritt (erst der Schalter "Aktivieren").
 */
export function nextApprovalStep(entry: TtsProviderEntry): ApprovalStep {
  if (entry.descriptor.opt_in && !entry.enabled) return null;
  if (entry.license_needs_ack && !entry.license_acked) return "license";
  if (entry.remote && !entry.allow_remote) return "remote";
  return null;
}

/** Hat der Anbieter einen Schalter "Aktivieren"? */
export function hasEnableSwitch(entry: TtsProviderEntry): boolean {
  return entry.descriptor.opt_in && entry.descriptor.tier !== "unavailable";
}

/** Darf der Anbieter ueberhaupt gewaehlt werden (nur Stufe, nicht Sperre)? */
export function isSelectableTier(entry: TtsProviderEntry): boolean {
  return (
    entry.descriptor.tier === "supported" ||
    entry.descriptor.tier === "experimental"
  );
}

/** Eine Zeile "Quelle" nur fuer echte Web-Adressen (kein `file:` o. ae.). */
export function safeSourceUrl(url: string): string | null {
  return /^https?:\/\//i.test(url.trim()) ? url.trim() : null;
}

// ---------------------------------------------------------------------------
// Konfiguration (JSON-Text der Befehle ttsProviderGetConfig/-SetConfig)
// ---------------------------------------------------------------------------

/**
 * Felder, die NICHT als Eingabe erscheinen: Freigaben wirken nur ueber den
 * Freigabe-Ablauf (die Konfiguration kann sie nicht setzen), Kennung und
 * Beschriftung gehoeren zur Identitaet, Lizenzangaben und der Name der
 * Schluessel-Umgebungsvariable sind verschachtelt bzw. gehoeren nicht hierher.
 */
export const HIDDEN_FIELDS: ReadonlySet<string> = new Set([
  "allow_remote",
  "allow_network",
  "id",
  "label",
  "license",
  "api_key_env",
]);

/** Zahlfelder (der JSON-Wert allein verraet bei `null` den Typ nicht). */
const NUMBER_FIELDS: ReadonlySet<string> = new Set(["threads", "sample_rate"]);

/** Felder mit festem Wertevorrat; `""` steht fuer "keine Angabe" (`null`). */
const CHOICE_FIELDS: Readonly<Record<string, readonly string[]>> = {
  quant: ["", "q8"],
  lang: ["de", "en", "fr", "es", "pt", "none"],
};

export type FieldKind = "text" | "number" | "bool" | "list" | "choice";

export interface ConfigField {
  key: string;
  kind: FieldKind;
  /** Nur bei `choice`. */
  choices?: readonly string[];
  /** Ein leeres Feld bedeutet `null` (Original war `null`). */
  nullable: boolean;
}

export type FieldValue = string | boolean;

export type ParsedConfig = {
  original: Record<string, unknown>;
  fields: ConfigField[];
  values: Record<string, FieldValue>;
};

function fieldOf(key: string, value: unknown): ConfigField | null {
  if (HIDDEN_FIELDS.has(key)) return null;
  const nullable = value === null;
  if (typeof value === "boolean") return { key, kind: "bool", nullable };
  if (Array.isArray(value) && value.every((v) => typeof v === "string"))
    return { key, kind: "list", nullable: false };
  if (CHOICE_FIELDS[key])
    return { key, kind: "choice", choices: CHOICE_FIELDS[key], nullable };
  if (typeof value === "number" || NUMBER_FIELDS.has(key))
    return { key, kind: "number", nullable };
  if (typeof value === "string" || value === null)
    return { key, kind: "text", nullable };
  // Verschachteltes (Objekte): nicht als Feld anbieten.
  return null;
}

function valueOf(field: ConfigField, raw: unknown): FieldValue {
  if (field.kind === "bool") return raw === true;
  if (field.kind === "list") return (raw as string[]).join(", ");
  if (raw === null || raw === undefined) return "";
  return String(raw);
}

/** Konfigurations-JSON in Felder und Eingabewerte zerlegen. */
export function parseConfig(json: string): ParsedConfig {
  const parsed: unknown = JSON.parse(json);
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed))
    throw new Error("Konfiguration ist kein Objekt");
  const original = parsed as Record<string, unknown>;
  const fields: ConfigField[] = [];
  const values: Record<string, FieldValue> = {};
  for (const [key, raw] of Object.entries(original)) {
    const field = fieldOf(key, raw);
    if (!field) continue;
    fields.push(field);
    values[key] = valueOf(field, raw);
  }
  return { original, fields, values };
}

export class ConfigInputError extends Error {
  constructor(
    public readonly key: string,
    message: string,
  ) {
    super(message);
  }
}

/**
 * Eingaben zurueck in das JSON: nicht angebotene Felder bleiben unveraendert
 * (auch die Freigaben -- das Backend ignoriert sie ohnehin).
 */
export function buildConfig(
  parsed: ParsedConfig,
  values: Record<string, FieldValue>,
): string {
  const out: Record<string, unknown> = { ...parsed.original };
  for (const field of parsed.fields) {
    const raw = values[field.key];
    switch (field.kind) {
      case "bool":
        out[field.key] = raw === true;
        break;
      case "list":
        out[field.key] = String(raw)
          .split(",")
          .map((s) => s.trim())
          .filter((s) => s.length > 0);
        break;
      case "number": {
        const text = String(raw).trim();
        if (text === "") {
          if (!field.nullable)
            throw new ConfigInputError(field.key, "number_required");
          out[field.key] = null;
        } else {
          const n = Number(text);
          if (!Number.isFinite(n))
            throw new ConfigInputError(field.key, "number_invalid");
          out[field.key] = n;
        }
        break;
      }
      default: {
        const text = String(raw).trim();
        out[field.key] = text === "" && field.nullable ? null : text;
      }
    }
  }
  return JSON.stringify(out);
}

// ---------------------------------------------------------------------------
// Eingabehilfen der Einstellungsmaske
// ---------------------------------------------------------------------------

/** Welche Art Auswahl ein Feld anbietet ("Durchsuchen ..."). */
export type PickKind = "file" | "folder";

/**
 * Felder eines lokalen Programms (Konfiguration mit `exe_path`) bekommen
 * "Durchsuchen ...": die Programmdatei per Dateiwahl, Modell und Hugging-Face-
 * Ordner per Ordnerwahl. Bei Anbietern ohne `exe_path` (z. B. `model` = Name
 * eines Modells auf einem Server) gibt es keinen Knopf -- dort waere ein Ordner
 * falsch.
 */
export function pickKind(
  key: string,
  fields: readonly ConfigField[],
): PickKind | null {
  if (!fields.some((f) => f.key === "exe_path")) return null;
  if (key === "exe_path") return "file";
  if (key === "model" || key === "hf_home") return "folder";
  return null;
}

/** Pflichtfelder eines lokalen Programms: ohne sie kann nichts starten. */
export function requiredFields(fields: readonly ConfigField[]): string[] {
  if (!fields.some((f) => f.key === "exe_path")) return [];
  return ["exe_path", "model"].filter((key) =>
    fields.some((f) => f.key === key),
  );
}

/**
 * Zu welchem Feld gehoert eine Meldung des Backends? Die Pruefmeldungen nennen
 * das Feld zuerst ("exe_path fehlt: ...", "model 'x' ...", "voice: ...").
 * `null`: keine Feldmeldung (allgemeiner Fehler).
 */
export function fieldOfMessage(
  message: string,
  keys: readonly string[],
): string | null {
  const first = /^([a-z][a-z0-9_]*)(?=[\s:'"]|$)/.exec(message.trim())?.[1];
  return first && keys.includes(first) ? first : null;
}

/** Stimmen die Eingaben mit dem gespeicherten Stand ueberein? */
export function sameValues(
  a: Record<string, FieldValue>,
  b: Record<string, FieldValue>,
): boolean {
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
  for (const key of keys) {
    if (String(a[key] ?? "") !== String(b[key] ?? "")) return false;
  }
  return true;
}

// ---------------------------------------------------------------------------
// Fehler
// ---------------------------------------------------------------------------

/**
 * Das Backend liefert deutsche, lesbare Meldungen; die Klasse (`kind`) gibt
 * nur die Ueberschrift vor. Ein unbekannter Fehler (kein `TtsError`) wird
 * als Text durchgereicht.
 */
export function asTtsError(error: unknown): TtsError {
  if (
    error !== null &&
    typeof error === "object" &&
    "kind" in error &&
    "message" in error
  ) {
    return error as TtsError;
  }
  return {
    kind: "backend",
    retryable: false,
    message: error instanceof Error ? error.message : String(error),
  };
}

/** Der Probetext, mit dem die Probestimme startet (deutsch). */
export const DEFAULT_PREVIEW_KEY = "tts.providers.preview.defaultText";

/** Wert fuer "keine Stimme gewaehlt" in der Auswahlliste. */
export const VOICE_DEFAULT = "@default";
/** Wert fuer "alle Sprachen" in der Sprachauswahl. */
export const LANG_ALL = "@all";

/** Aktiv heisst: die Probestimme laeuft noch (Abbrechen ist sinnvoll). */
export function previewIsActive(phase: string | undefined): boolean {
  return phase === "starting" || phase === "started" || phase === "first_audio";
}

// ---------------------------------------------------------------------------
// Sichtbare Stimmen im Vorlesen-Dropdown (Spiegel von `policy::visible_voice`
// und `commands::tts_providers::voice_options` im Backend)
// ---------------------------------------------------------------------------

/** Kennung der Standardstimme eines Anbieters: immer im Dropdown, nie abwaehlbar. */
export const PROVIDER_DEFAULT_VOICE = "default";

/** Eingebaute Engines haben ihre eigenen Eintraege im Dropdown. */
const BUILTIN_PROVIDERS: ReadonlySet<string> = new Set(["fish", "piper"]);

/**
 * Bietet der Anbieter einzelne Stimmen im Vorlesen-Dropdown an (und damit eine
 * Auswahl "im Vorlesen anzeigen")? Alle freischaltbaren Anbieter ausser den
 * eingebauten; Mock und gesperrte Stufen nie. Gleiche Regel wie im Backend.
 */
export function offersDropdownVoices(entry: TtsProviderEntry): boolean {
  return !BUILTIN_PROVIDERS.has(entry.descriptor.id) && isSelectableTier(entry);
}

/**
 * Steht die Stimme im Dropdown? `visible`: die gespeicherte Liste des Anbieters
 * (leer = alle), `selected`: die gerade gewaehlte Vorlese-Stimme des Anbieters.
 */
export function isVoiceShown(
  visible: readonly string[],
  selected: string | null,
  id: string,
): boolean {
  return (
    id === PROVIDER_DEFAULT_VOICE ||
    id === selected ||
    visible.length === 0 ||
    visible.includes(id)
  );
}

/**
 * Die neue gespeicherte Liste nach einem Haken: Sind danach alle Stimmen
 * sichtbar, wird nichts gespeichert (leer = alle, auch fuer spaeter
 * hinzukommende Stimmen); ist keine benannte Stimme mehr sichtbar, steht nur
 * die Standardstimme in der Liste ("keine").
 */
export function toggleVisibleVoice(
  visible: readonly string[],
  allIds: readonly string[],
  id: string,
  shown: boolean,
): string[] {
  const current = new Set(visible.length === 0 ? allIds : visible);
  if (shown) current.add(id);
  else current.delete(id);
  const named = allIds.filter((v) => current.has(v));
  if (named.length === allIds.length) return [];
  return named.length === 0 ? [PROVIDER_DEFAULT_VOICE] : named;
}

/** Wert des Vorlesen-Dropdowns fuer eine Stimme eines Anbieters: `provider:<id>:<stimme>`. */
export function providerVoiceValue(providerId: string, voice: string): string {
  return `provider:${providerId}:${voice}`;
}

/**
 * Zerlegt `provider:<id>:<stimme>` (Stimme darf `:` enthalten); `provider:<id>`
 * ohne Stimme ergibt die Standardstimme. Alles andere: `null`.
 */
export function parseProviderVoiceValue(
  value: string,
): { providerId: string; voice: string } | null {
  if (!value.startsWith("provider:")) return null;
  const rest = value.slice("provider:".length);
  const cut = rest.indexOf(":");
  const providerId = cut < 0 ? rest : rest.slice(0, cut);
  if (!providerId) return null;
  const voice = cut < 0 ? "" : rest.slice(cut + 1);
  return { providerId, voice: voice || PROVIDER_DEFAULT_VOICE };
}

/** Name vor der Kurzbeschreibung: "Nele – Rounded, open tone…" -> "Nele". */
export function shortVoiceName(label: string): string {
  const name = label.split(" – ")[0].trim();
  return name || label;
}
