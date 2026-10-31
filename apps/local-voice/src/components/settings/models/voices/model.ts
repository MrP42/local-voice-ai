import type { Device, TtsProviderEntry } from "@/bindings";
import type { VoiceTryTarget } from "../../tts/providers/voiceTry";

/**
 * Reine Logik der Stimmenverwaltung (ohne Oberflaeche, ohne Tauri): das
 * Stimmenmodell ueber alle Anbieter, Filter mit Zaehlern, Sortierung und der
 * Zustand eines Anbieters. Alles wird aus dem abgeleitet, was die Befehle schon
 * liefern; was es nicht gibt (Qualitaet, Rechenaufwand), steht nicht darin.
 */

/** Zustand einer Stimme aus Sicht der Verwaltung. */
export type VoiceState =
  /** Nutzbar: Stimme da, Anbieter bereit. */
  | "ready"
  /** Piper-Stimme, noch nicht geladen. */
  | "load"
  /** Piper-Stimme wird gerade geladen. */
  | "downloading"
  /** Piper-Stimme geladen, aber das Piper-Programm fehlt. */
  | "runtime"
  /** Der Anbieter ist nicht bereit oder gesperrt. */
  | "setup"
  /** Der Anbieter oder die Stimme ist auf diesem System nicht unterstuetzt. */
  | "unsupported";

export type LicenseKind = "free" | "nonCommercial" | "unverified";
export type DeviceGroup = "cpu" | "gpu" | "server";

export interface VoiceItem {
  /** Eindeutig ueber alle Anbieter. */
  key: string;
  /** Wert der Stimmenwahl (`@default`, `<fish-id>`, `piper:<id>`, `provider:<id>:<stimme>`). */
  value: string;
  providerId: string;
  providerLabel: string;
  name: string;
  description: string | null;
  /** Sprachkuerzel; `und` = mehrsprachig. */
  language: string;
  gender: string | null;
  devices: DeviceGroup[];
  license: LicenseKind;
  /** Lizenzname der Stimme, wenn bekannt (z. B. `CC0-1.0`). */
  licenseName: string | null;
  state: VoiceState;
  sizeMb: number | null;
  /** Kennung des Downloads (nur Piper); `null` ohne Download. */
  downloadId: string | null;
  /** Probe erzeugen (Fish, Piper, Anbieter ohne fertige Hoerprobe). */
  tryTarget: VoiceTryTarget | null;
  /** Fertige Hoerprobe: Pfad, `null` = entsteht noch, `undefined` = keine vorgesehen. */
  samplePath: string | null | undefined;
  /** Der Anbieter bietet die Stimme im Vorlesen-Dropdown nicht an (abgewaehlt). */
  hiddenInReadAloud: boolean;
  /** Sie ist die Stimme, die gerade vorliest. */
  selected: boolean;
}

export function deviceGroup(device: Device): DeviceGroup {
  if (device === "cpu") return "cpu";
  if (device === "remote") return "server";
  return "gpu";
}

export function deviceGroups(devices: readonly Device[]): DeviceGroup[] {
  return [...new Set(devices.map(deviceGroup))];
}

/** Wie frei darf die Lizenz genutzt werden? Stimmen-Lizenz vor Anbieter-Lizenz. */
export function licenseKind(
  nonCommercial: boolean,
  verified: boolean,
): LicenseKind {
  if (nonCommercial) return "nonCommercial";
  return verified ? "free" : "unverified";
}

export type ProviderStatus =
  "ready" | "notReady" | "blocked" | "notEnabled" | "unavailable";

/** Zustand eines Anbieters fuer Karte und Filter. */
export function providerStatus(entry: TtsProviderEntry): ProviderStatus {
  if (entry.descriptor.tier === "unavailable") return "unavailable";
  if (entry.blocked !== null) {
    return entry.descriptor.opt_in && !entry.enabled ? "notEnabled" : "blocked";
  }
  return entry.health.available ? "ready" : "notReady";
}

/** Anbieter, die gerade nicht bereit sind (Stimmen stehen, sind aber nicht waehlbar). */
export function providerIsReady(entry: TtsProviderEntry): boolean {
  return providerStatus(entry) === "ready";
}

// ---------------------------------------------------------------------------
// Filter
// ---------------------------------------------------------------------------

export interface Filters {
  languages: ReadonlySet<string>;
  devices: ReadonlySet<DeviceGroup>;
  providers: ReadonlySet<string>;
  licenses: ReadonlySet<LicenseKind>;
  onlyInstalled: boolean;
  query: string;
}

export const EMPTY_FILTERS: Filters = {
  languages: new Set(),
  devices: new Set(),
  providers: new Set(),
  licenses: new Set(),
  onlyInstalled: false,
  query: "",
};

export type FacetKey = "languages" | "devices" | "providers" | "licenses";

/** Wie viele Auswahlen sind gesetzt (fuer den Knopf "Filter (n)")? */
export function activeFilterCount(f: Filters): number {
  return (
    f.languages.size +
    f.devices.size +
    f.providers.size +
    f.licenses.size +
    (f.onlyInstalled ? 1 : 0)
  );
}

const installed = (v: VoiceItem) => v.state === "ready";

function matches(v: VoiceItem, f: Filters, skip?: FacetKey): boolean {
  if (skip !== "languages" && f.languages.size && !f.languages.has(v.language))
    return false;
  if (
    skip !== "devices" &&
    f.devices.size &&
    !v.devices.some((d) => f.devices.has(d))
  )
    return false;
  if (
    skip !== "providers" &&
    f.providers.size &&
    !f.providers.has(v.providerId)
  )
    return false;
  if (skip !== "licenses" && f.licenses.size && !f.licenses.has(v.license))
    return false;
  if (f.onlyInstalled && !installed(v)) return false;
  const q = f.query.trim().toLowerCase();
  if (
    q &&
    !`${v.name} ${v.providerLabel} ${v.description ?? ""} ${v.language}`
      .toLowerCase()
      .includes(q)
  )
    return false;
  return true;
}

export function applyFilters(items: readonly VoiceItem[], f: Filters) {
  return items.filter((v) => matches(v, f));
}

/**
 * Zaehler einer Gruppe: Treffer unter allen ANDEREN Filtern, damit man sieht,
 * was ein weiterer Haken brachte. Die Gruppe selbst bleibt dabei unberuecksichtigt.
 */
export function facetCounts(
  items: readonly VoiceItem[],
  f: Filters,
  facet: FacetKey,
): Map<string, number> {
  const counts = new Map<string, number>();
  const bump = (key: string) => counts.set(key, (counts.get(key) ?? 0) + 1);
  for (const v of items) {
    if (!matches(v, f, facet)) continue;
    if (facet === "languages") bump(v.language);
    else if (facet === "devices") v.devices.forEach(bump);
    else if (facet === "providers") bump(v.providerId);
    else bump(v.license);
  }
  return counts;
}

/** Anzahl installierter Stimmen unter allen anderen Filtern (Zaehler am Haken). */
export function installedCount(items: readonly VoiceItem[], f: Filters) {
  return items.filter(
    (v) => installed(v) && matches(v, { ...f, onlyInstalled: false }),
  ).length;
}

// ---------------------------------------------------------------------------
// Sortierung
// ---------------------------------------------------------------------------

export type SortKey = "recommended" | "name" | "language" | "provider";
export const SORT_KEYS: readonly SortKey[] = [
  "recommended",
  "name",
  "language",
  "provider",
];

const STATE_RANK: Record<VoiceState, number> = {
  ready: 0,
  downloading: 1,
  load: 2,
  runtime: 3,
  setup: 4,
  unsupported: 5,
};

export function sortVoices(
  items: readonly VoiceItem[],
  key: SortKey,
  locale: string,
): VoiceItem[] {
  const byName = (a: VoiceItem, b: VoiceItem) =>
    a.name.localeCompare(b.name, locale, { sensitivity: "base" });
  const cmp: Record<SortKey, (a: VoiceItem, b: VoiceItem) => number> = {
    recommended: (a, b) =>
      Number(b.selected) - Number(a.selected) ||
      STATE_RANK[a.state] - STATE_RANK[b.state] ||
      byName(a, b),
    name: byName,
    language: (a, b) =>
      a.language.localeCompare(b.language, locale) || byName(a, b),
    provider: (a, b) =>
      a.providerLabel.localeCompare(b.providerLabel, locale) || byName(a, b),
  };
  return [...items].sort(cmp[key]);
}

/** Kurzname: "Nele – Rounded, open tone…" -> Name und Beschreibung getrennt. */
export function splitLabel(label: string): {
  name: string;
  description: string | null;
} {
  const [name, ...rest] = label.split(" – ");
  const description = rest.join(" – ").trim();
  return { name: name.trim() || label, description: description || null };
}
