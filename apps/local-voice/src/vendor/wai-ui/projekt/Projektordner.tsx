import { createElement, useEffect, useId, useMemo, useRef, useState } from "react";
import { Button } from "../components/Button";
import { Drawer } from "../components/Drawer";
import { Badge } from "../components/StatusChip";
import { cx } from "../cx";
import { Icon, type IconName } from "../icons";
import { type Datenklasse, type EintragArt, istSensibel } from "./typen";

export interface ProjektKurz {
  id: string;
  title: string;
  dataClass: Datenklasse;
  /** Art.-9-Kennzeichen (`special_category`). */
  specialCategory?: boolean;
  archived?: boolean;
  /** Web ist für dieses Projekt freigegeben (`web_research_enabled`). */
  webResearchEnabled?: boolean;
  entryCount: number;
}

export interface ExterneKopie {
  target: "export" | "presenter" | "exchange_folder";
  /** Zeitpunkt als fertig formulierter Text („30.10.“). */
  atLabel: string;
  label?: string;
}

export interface ProjektEintrag {
  id: string;
  kind: EintragArt;
  /** Fehlt der Titel (oder ist `available` false), zeigt die Liste nur den Platzhalter. */
  title?: string;
  /** Kurzbeschreibung unter dem Titel („Recherche · Gründlich · 9 Quellen“). */
  subtitle?: string;
  /** Herkunftsmodul (`origin_app`), z. B. `corporate-llm`. */
  originApp: string;
  dataClass: Datenklasse;
  /** Kein Leserecht in der Herkunftsapp: Platzhalter ohne Titel und Inhalt. */
  available?: boolean;
  /** Datum als fertig formulierter Text („heute 20:41“). */
  dateLabel?: string;
  /** Vorlesefortschritt 0 bis 1. */
  readProgress?: number;
  readDetail?: string;
  externalCopies?: readonly ExterneKopie[];
}

export type ProjektAktion =
  | "new-project"
  | "start-research"
  | "chat"
  | "open"
  | "read-aloud"
  | "export"
  | "reassign";

export interface ProjektordnerLabels {
  projects: string;
  newProject: string;
  switchProject: string;
  entryCount: (n: number) => string;
  archived: string;
  webReleased: string;
  specialCategory: string;
  startResearch: string;
  chatInProject: string;
  details: string;
  closeDetails: string;
  detailsTitle: string;
  filterLabel: string;
  all: string;
  search: string;
  entries: string;
  noEntries: string;
  noHits: string;
  unavailable: string;
  unavailableReason: string;
  pickEntry: string;
  fieldTitle: string;
  fieldOrigin: string;
  fieldClass: string;
  fieldReading: string;
  fieldCopies: string;
  readingProgress: (percent: number) => string;
  noCopies: string;
  noReading: string;
  open: string;
  readAloud: string;
  exportEntry: string;
  reassign: string;
  copyTargets: Record<"export" | "presenter" | "exchange_folder", string>;
  groups: Record<ArtGruppe, string>;
  classes: Record<Datenklasse, string>;
  kinds: Record<EintragArt, string>;
  apps: Record<string, string>;
}

export type ArtGruppe =
  | "meetings"
  | "conversations"
  | "books"
  | "research"
  | "notes"
  | "talks"
  | "documents"
  | "plans";

const GRUPPEN: readonly (readonly [ArtGruppe, readonly EintragArt[]])[] = [
  ["meetings", ["meeting", "transcript"]],
  ["conversations", ["conversation"]],
  ["books", ["book", "book_document"]],
  ["research", ["research"]],
  ["notes", ["note"]],
  ["talks", ["talk"]],
  ["documents", ["document"]],
  ["plans", ["plan"]],
];

const ART_ICON: Record<EintragArt, IconName> = {
  meeting: "calendar",
  transcript: "waveform",
  conversation: "chat",
  book: "list",
  book_document: "file",
  note: "edit",
  research: "search",
  document: "file",
  plan: "board",
  talk: "mic",
};

export const PROJEKTORDNER_LABELS: ProjektordnerLabels = {
  projects: "Projekte",
  newProject: "Neues Projekt",
  switchProject: "Projekt wechseln",
  entryCount: (n) => (n === 1 ? "1 Eintrag" : `${n} Einträge`),
  archived: "archiviert",
  webReleased: "Web freigegeben",
  specialCategory: "besondere Kategorie",
  startResearch: "Recherche starten",
  chatInProject: "Chat im Projekt",
  details: "Details",
  closeDetails: "Details schließen",
  detailsTitle: "Eintrag",
  filterLabel: "Nach Art filtern",
  all: "Alle",
  search: "Im Projekt suchen",
  entries: "Einträge",
  noEntries: "In diesem Projekt liegen noch keine Einträge.",
  noHits: "Keine Einträge zu dieser Auswahl.",
  unavailable: "Nicht verfügbar",
  unavailableReason: "Kein Leserecht in der Herkunftsapp",
  pickEntry: "Wählen Sie einen Eintrag, um Herkunft und Fortschritt zu sehen.",
  fieldTitle: "Titel",
  fieldOrigin: "Herkunft",
  fieldClass: "Datenklasse",
  fieldReading: "Vorlesen",
  fieldCopies: "Kopien außerhalb",
  readingProgress: (p) => `Vorlesefortschritt ${p} %`,
  noCopies: "Keine",
  noReading: "Noch nicht vorgelesen",
  open: "Öffnen",
  readAloud: "Vorlesen",
  exportEntry: "Exportieren",
  reassign: "Zuordnung ändern",
  copyTargets: {
    export: "Export",
    presenter: "Presenter",
    exchange_folder: "Austauschordner",
  },
  groups: {
    meetings: "Besprechungen",
    conversations: "Unterhaltungen",
    books: "Bücher",
    research: "Recherchen",
    notes: "Notizen",
    talks: "Vorträge",
    documents: "Dokumente",
    plans: "Pläne",
  },
  classes: {
    PUBLIC: "öffentlich",
    INTERNAL: "intern",
    PERSONAL: "personenbezogen",
    SPECIAL: "besondere Kategorie",
    RESTRICTED: "eingeschränkt",
  },
  kinds: {
    meeting: "Besprechung",
    transcript: "Transkript",
    conversation: "Unterhaltung",
    book: "Buch",
    book_document: "Buchdokument",
    note: "Notiz",
    research: "Recherche",
    document: "Dokument",
    plan: "Plan",
    talk: "Vortrag",
  },
  apps: {
    "corporate-llm": "Corporate LLM",
    lva: "Local Voice AI",
    presenter: "Presenter",
    planner: "Planner",
    "team-chat": "Team-Chat",
  },
};

export interface ProjektordnerProps {
  projects: readonly ProjektKurz[];
  selectedProjectId: string;
  onSelectProject?: (id: string) => void;
  entries: readonly ProjektEintrag[];
  /** Ausgewählter Eintrag (kontrolliert); ohne Angabe verwaltet der Baustein die Auswahl selbst. */
  selectedEntryId?: string;
  onSelectEntry?: (id: string) => void;
  /** Schaltflächen im Kopf und im Detailbereich; der Baustein ändert selbst nichts. */
  onAction?: (action: ProjektAktion, entryId?: string) => void;
  /** Überschriftenebene des Projekttitels (Standard 2; es gibt nur ein h1 je Seite im Seitenkopf). */
  headingLevel?: 2 | 3 | 4;
  /**
   * Detailbereich als feste Spalte (ab 1100 px Breite) oder als Seitenbereich mit Fokusfalle (darunter).
   * `auto` misst die Breite des Bausteins; die anderen Werte erzwingen eine Darstellung (Tests, Stories).
   */
  detailMode?: "auto" | "spalte" | "seitenbereich";
  /**
   * Nur dieses Projekt zeigen (Einbettung in eine Projektseite): Projektliste, Projektwechsler und
   * „Neues Projekt“ entfallen, der Inhalt nutzt die volle Breite.
   */
  singleProject?: boolean;
  /** Angebotene Aktionen als Knöpfe (Standard: alle); nicht angebotene Aktionen erscheinen nicht. */
  actions?: readonly ProjektAktion[];
  labels?: Partial<ProjektordnerLabels>;
  className?: string;
}

/** Ab dieser Breite steht der Detailbereich als feste Spalte neben der Liste. */
const DETAIL_SPALTE_AB = 1100;

const istVerfuegbar = (e: ProjektEintrag) => e.available !== false && Boolean(e.title);

export function Projektordner({
  projects,
  selectedProjectId,
  onSelectProject,
  entries,
  selectedEntryId,
  onSelectEntry,
  onAction,
  headingLevel = 2,
  detailMode = "auto",
  singleProject = false,
  actions,
  labels: overrides,
  className,
}: ProjektordnerProps) {
  const L = useMemo(() => ({ ...PROJEKTORDNER_LABELS, ...overrides }), [overrides]);
  const ids = useId();
  const [gruppe, setGruppe] = useState<ArtGruppe | "all">("all");
  const [suche, setSuche] = useState("");
  const [eigeneWahl, setEigeneWahl] = useState<string | undefined>();
  const [detailOffen, setDetailOffen] = useState(false);
  const [schmal, setSchmal] = useState(false);
  const wurzel = useRef<HTMLDivElement>(null);
  const waehlbar = selectedEntryId ?? eigeneWahl;

  const projekt = projects.find((p) => p.id === selectedProjectId);
  const apps = L.apps;
  const appName = (slug: string) => apps[slug] ?? slug;

  const zaehler = useMemo(() => {
    const m = new Map<ArtGruppe, number>();
    for (const e of entries) {
      if (!istVerfuegbar(e)) continue;
      const g = GRUPPEN.find(([, arten]) => arten.includes(e.kind))?.[0];
      if (g) m.set(g, (m.get(g) ?? 0) + 1);
    }
    return m;
  }, [entries]);

  const sichtbar = useMemo(() => {
    const arten = gruppe === "all" ? null : GRUPPEN.find(([g]) => g === gruppe)?.[1];
    const q = suche.trim().toLocaleLowerCase("de");
    return entries.filter((e) => {
      if (arten && (!istVerfuegbar(e) || !arten.includes(e.kind))) return false;
      if (!q) return true;
      if (!istVerfuegbar(e)) return false;
      return `${e.title} ${e.subtitle ?? ""} ${apps[e.originApp] ?? e.originApp}`
        .toLocaleLowerCase("de")
        .includes(q);
    });
  }, [entries, gruppe, suche, apps]);

  const gewaehlt = entries.find((e) => e.id === waehlbar && istVerfuegbar(e));

  useEffect(() => {
    const el = wurzel.current;
    if (!el || detailMode !== "auto" || typeof ResizeObserver === "undefined") return;
    const messen = () => {
      const breite = el.getBoundingClientRect().width;
      if (breite > 0) setSchmal(breite < DETAIL_SPALTE_AB);
    };
    messen();
    const beobachter = new ResizeObserver(messen);
    beobachter.observe(el);
    return () => beobachter.disconnect();
  }, [detailMode]);

  const seitenbereich = detailMode === "auto" ? schmal : detailMode === "seitenbereich";
  const waehle = (id: string) => {
    setEigeneWahl(id);
    onSelectEntry?.(id);
  };

  const klasseText = (k: Datenklasse) => L.classes[k];
  const darf = (a: ProjektAktion) => !actions || actions.includes(a);
  const titel = createElement(
    `h${headingLevel}`,
    { className: "wai-projekt__titel" },
    projekt?.title ?? "",
  );

  const detailInhalt = gewaehlt ? (
    <EintragDetail
      eintrag={gewaehlt}
      labels={L}
      appName={appName}
      onAction={(a) => onAction?.(a, gewaehlt.id)}
      darf={darf}
    />
  ) : (
    <p className="wai-projekt__leer">{L.pickEntry}</p>
  );

  return (
    <div
      ref={wurzel}
      className={cx("wai-projekt", className)}
      data-detail={seitenbereich ? "seitenbereich" : "spalte"}
      data-einzeln={singleProject ? "" : undefined}
    >
      <div className="wai-projekt__raster">
        {!singleProject && (
          <nav className="wai-projekt__liste" aria-label={L.projects}>
            <div className="wai-projekt__kopf2">
              <span className="wai-projekt__zwischen">{L.projects}</span>
              <Button
                variant="subtle"
                size="small"
                aria-label={L.newProject}
                onPress={() => onAction?.("new-project")}
              >
                <Icon name="plus" />
              </Button>
            </div>
            <ul className="wai-projekt__projekte">
              {projects.map((p) => (
                <li key={p.id}>
                  <button
                    type="button"
                    className="wai-projekt__projekt"
                    aria-current={p.id === selectedProjectId ? "true" : undefined}
                    onClick={() => onSelectProject?.(p.id)}
                  >
                    <span className="wai-projekt__projekttext">
                      <span>{p.title}</span>
                      <small>
                        {L.entryCount(p.entryCount)}
                        {p.archived ? ` · ${L.archived}` : ""}
                      </small>
                    </span>
                    {istSensibel(p.dataClass) && (
                      <Badge tone="warning">{klasseText(p.dataClass)}</Badge>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          </nav>
        )}

        <section className="wai-projekt__mitte" aria-label={projekt?.title ?? L.entries}>
          {!singleProject && (
            <div className="wai-projekt__wechsler">
              <label htmlFor={`${ids}-wahl`} className="wai-projekt__nur-sr">
                {L.switchProject}
              </label>
              <select
                id={`${ids}-wahl`}
                className="wai-projekt__wahl"
                value={selectedProjectId}
                onChange={(e) => onSelectProject?.(e.target.value)}
              >
                {projects.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.title}
                    {istSensibel(p.dataClass) ? ` (${klasseText(p.dataClass)})` : ""}
                    {p.archived ? ` (${L.archived})` : ""}
                  </option>
                ))}
              </select>
            </div>
          )}
          <header className="wai-projekt__kopf">
            {titel}
            {projekt && (
              <span className="wai-projekt__marken">
                <Badge tone={istSensibel(projekt.dataClass) ? "warning" : "neutral"}>
                  {klasseText(projekt.dataClass)}
                </Badge>
                {projekt.specialCategory && <Badge tone="warning">{L.specialCategory}</Badge>}
                {projekt.archived && <Badge>{L.archived}</Badge>}
                {projekt.webResearchEnabled && <Badge tone="info">{L.webReleased}</Badge>}
              </span>
            )}
            <span className="wai-projekt__aktionen">
              {darf("start-research") && (
                <Button variant="primary" onPress={() => onAction?.("start-research")}>
                  {L.startResearch}
                </Button>
              )}
              {darf("chat") && (
                <Button onPress={() => onAction?.("chat")}>{L.chatInProject}</Button>
              )}
              {seitenbereich && (
                <Button
                  className="wai-projekt__detailknopf"
                  aria-expanded={detailOffen}
                  aria-haspopup="dialog"
                  onPress={() => setDetailOffen(true)}
                >
                  {L.details}
                </Button>
              )}
            </span>
          </header>

          <div className="wai-projekt__leiste">
            {/* biome-ignore lint/a11y/useSemanticElements: Umschaltgruppe ohne fieldset-Rahmen */}
            <div role="group" aria-label={L.filterLabel} className="wai-projekt__chips">
              <button
                type="button"
                className="wai-projekt__chip"
                aria-pressed={gruppe === "all"}
                onClick={() => setGruppe("all")}
              >
                {L.all} {entries.length}
              </button>
              {GRUPPEN.filter(([g]) => zaehler.has(g)).map(([g]) => (
                <button
                  key={g}
                  type="button"
                  className="wai-projekt__chip"
                  aria-pressed={gruppe === g}
                  onClick={() => setGruppe(g)}
                >
                  {L.groups[g]} {zaehler.get(g)}
                </button>
              ))}
            </div>
            <input
              type="search"
              className="wai-projekt__suche"
              placeholder={L.search}
              aria-label={L.search}
              value={suche}
              onChange={(e) => setSuche(e.target.value)}
            />
          </div>

          {sichtbar.length === 0 ? (
            <p className="wai-projekt__leer">{entries.length === 0 ? L.noEntries : L.noHits}</p>
          ) : (
            <ul className="wai-projekt__eintraege" aria-label={L.entries}>
              {sichtbar.map((e) => (
                <li key={e.id}>
                  {istVerfuegbar(e) ? (
                    <button
                      type="button"
                      className="wai-projekt__zeile"
                      aria-current={e.id === waehlbar ? "true" : undefined}
                      onClick={() => waehle(e.id)}
                    >
                      <span className="wai-projekt__art" role="img" aria-label={L.kinds[e.kind]}>
                        <Icon name={ART_ICON[e.kind]} />
                      </span>
                      <span className="wai-projekt__text">
                        <span className="wai-projekt__eintragstitel">{e.title}</span>
                        {e.subtitle && <small>{e.subtitle}</small>}
                      </span>
                      <Badge>{appName(e.originApp)}</Badge>
                      {e.dateLabel && <span className="wai-projekt__datum">{e.dateLabel}</span>}
                    </button>
                  ) : (
                    <div className="wai-projekt__zeile wai-projekt__zeile--leer">
                      <span className="wai-projekt__art" aria-hidden="true">
                        <Icon name="lock" />
                      </span>
                      <span className="wai-projekt__text">
                        <span className="wai-projekt__eintragstitel wai-projekt__na">
                          {L.unavailable}
                        </span>
                        <small>{L.unavailableReason}</small>
                      </span>
                      <Badge>{appName(e.originApp)}</Badge>
                      <span className="wai-projekt__datum">—</span>
                    </div>
                  )}
                </li>
              ))}
            </ul>
          )}
        </section>

        {seitenbereich ? (
          <Drawer
            isOpen={detailOffen}
            onOpenChange={setDetailOffen}
            title={L.detailsTitle}
            closeLabel={L.closeDetails}
          >
            {detailInhalt}
          </Drawer>
        ) : (
          <aside id={`${ids}-detail`} className="wai-projekt__detail" aria-label={L.detailsTitle}>
            <div className="wai-projekt__kopf2">
              <span className="wai-projekt__zwischen">{L.detailsTitle}</span>
            </div>
            {detailInhalt}
          </aside>
        )}
      </div>
    </div>
  );
}

function EintragDetail({
  eintrag: e,
  labels: L,
  appName,
  onAction,
  darf,
}: {
  eintrag: ProjektEintrag;
  labels: ProjektordnerLabels;
  appName: (slug: string) => string;
  onAction: (a: ProjektAktion) => void;
  darf: (a: ProjektAktion) => boolean;
}) {
  const prozent =
    e.readProgress === undefined
      ? undefined
      : Math.round(Math.min(1, Math.max(0, e.readProgress)) * 100);
  const kopien = e.externalCopies ?? [];
  return (
    <div className="wai-projekt__felder">
      <dl>
        <div>
          <dt>{L.fieldTitle}</dt>
          <dd>{e.title}</dd>
        </div>
        <div>
          <dt>{L.fieldOrigin}</dt>
          <dd>
            {appName(e.originApp)} · {L.kinds[e.kind]}
          </dd>
        </div>
        <div>
          <dt>{L.fieldClass}</dt>
          <dd>{L.classes[e.dataClass]}</dd>
        </div>
        <div>
          <dt>{L.fieldReading}</dt>
          <dd>
            {prozent === undefined ? (
              <small className="wai-projekt__na">{L.noReading}</small>
            ) : (
              <>
                <div
                  className="wai-projekt__fortschritt"
                  role="progressbar"
                  aria-label={L.fieldReading}
                  aria-valuetext={L.readingProgress(prozent)}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={prozent}
                >
                  <i style={{ inlineSize: `${prozent}%` }} />
                </div>
                <small>{e.readDetail ?? `${prozent} %`}</small>
              </>
            )}
          </dd>
        </div>
        <div>
          <dt>{L.fieldCopies}</dt>
          <dd>
            {kopien.length === 0
              ? L.noCopies
              : kopien.map((k) => `${k.label ?? L.copyTargets[k.target]} ${k.atLabel}`).join(" · ")}
          </dd>
        </div>
      </dl>
      <div className="wai-projekt__detailaktionen">
        {darf("open") && (
          <Button variant="primary" onPress={() => onAction("open")}>
            {L.open}
          </Button>
        )}
        {darf("read-aloud") && (
          <Button onPress={() => onAction("read-aloud")}>{L.readAloud}</Button>
        )}
        {darf("export") && <Button onPress={() => onAction("export")}>{L.exportEntry}</Button>}
        {darf("reassign") && <Button onPress={() => onAction("reassign")}>{L.reassign}</Button>}
      </div>
    </div>
  );
}
