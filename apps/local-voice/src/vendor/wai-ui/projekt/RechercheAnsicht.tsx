import { useId, useMemo, useRef, useState } from "react";
import { Button } from "../components/Button";
import { cx } from "../cx";
import { Icon } from "../icons";
import type { QuellenArt, QuellenUmfang, RechercheModus } from "./typen";

/** Satz oder Absatz der Antwort (`answer[]`); `citations` zählen ab 1 in `sources`. */
export interface AntwortTeil {
  text: string;
  citations: readonly number[];
  /** Nach der Prüfung als „ohne Quelle“ markiert. */
  unsupported: boolean;
  /** Nur schwacher Beleg; ohne Angabe schwach, wenn alle Belege Foren oder unbekannte Quellen sind. */
  weak?: boolean;
}

export interface RechercheQuelle {
  index: number;
  title: string;
  /** Anzeigeort („eur-lex.europa.eu“) – die Adresse selbst bleibt dem Aufrufer. */
  host: string;
  sourceType: QuellenArt;
  /** Abrufzeit als fertig formulierter Text („abgerufen 21:39“). */
  retrievedLabel: string;
  excerpt: string;
}

export interface Widerspruch {
  statement: string;
  sourceIndices: readonly number[];
}

export interface RechercheErgebnis {
  title?: string;
  answer: readonly AntwortTeil[];
  sources: readonly RechercheQuelle[];
  contradictions?: readonly Widerspruch[];
  followUps: readonly string[];
  /** Kennzeichnung unter der Antwort: Modus, Aufwand, Stand, Prüfstatus als fertiger Text. */
  provenance: string;
}

export interface RechercheSchritt {
  label: string;
  state: "done" | "running";
}

export interface RechercheAufwand {
  seconds: number;
  maxSources: number;
  model: string;
}

export type RechercheAktion = "read-aloud" | "report" | "export" | "presenter";

export interface RechercheAnsichtLabels {
  question: string;
  inProject: (title: string) => string;
  mode: string;
  modes: Record<RechercheModus, string>;
  scope: string;
  scopes: Record<QuellenUmfang, string>;
  webLocked: string;
  effort: (a: RechercheAufwand) => string;
  submit: string;
  busy: string;
  answer: string;
  noSource: string;
  weakProof: string;
  weakNote: string;
  contradiction: string;
  contradictionRefs: (idx: readonly number[]) => string;
  aiLabel: string;
  actions: Record<RechercheAktion, string>;
  followUp: string;
  onlyHere: string;
  sources: (n: number) => string;
  footnote: (n: number) => string;
  highlighted: (n: number, title: string) => string;
  sourceTypes: Record<QuellenArt, string>;
  steps: string;
  noResult: string;
}

export const RECHERCHE_LABELS: RechercheAnsichtLabels = {
  question: "Frage",
  inProject: (t) => `im Projekt „${t}“`,
  mode: "Modus",
  modes: { quick: "Schnell", thorough: "Gründlich", deep: "Tief" },
  scope: "Quellen",
  scopes: { web: "Web", project: "Projekt", both: "Beides" },
  webLocked: "Das Web ist für dieses Projekt nicht freigegeben.",
  effort: (a) => `ca. ${a.seconds} s · bis ${a.maxSources} Quellen · ${a.model}`,
  submit: "Recherchieren",
  busy: "Recherche läuft",
  answer: "Antwort",
  noSource: "ohne Quelle",
  weakProof: "nur schwacher Beleg",
  weakNote: "schwacher Beleg",
  contradiction: "Widerspruch",
  contradictionRefs: (i) => `(Quellen ${i.join(", ")})`,
  aiLabel: "KI-erstellt",
  actions: {
    "read-aloud": "Vorlesen",
    report: "Als Bericht (Vorlage)",
    export: "Exportieren",
    presenter: "An Presenter",
  },
  followUp: "Weiterfragen",
  onlyHere: "Nur in diesen Quellen weiterfragen",
  sources: (n) => `Quellen · ${n}`,
  footnote: (n) => `Quelle ${n}`,
  highlighted: (n, t) => `Quelle ${n} hervorgehoben: ${t}`,
  sourceTypes: {
    official: "amtlich",
    scholarly: "wissenschaftlich",
    press: "Presse",
    forum: "Forum",
    unknown: "Quelle unbekannt",
  },
  steps: "Ablauf",
  noResult: "Noch keine Recherche. Stellen Sie eine Frage.",
};

export interface RechercheAnsichtProps {
  projectTitle?: string;
  question?: string;
  onQuestionChange?: (q: string) => void;
  mode?: RechercheModus;
  onModeChange?: (m: RechercheModus) => void;
  scope?: QuellenUmfang;
  onScopeChange?: (s: QuellenUmfang) => void;
  /** Web ist im Projekt freigegeben; sonst sind „Web“ und „Beides“ gesperrt (Standard ja). */
  webEnabled?: boolean;
  effort?: RechercheAufwand;
  onSubmit?: (q: { question: string; mode: RechercheModus; scope: QuellenUmfang }) => void;
  busy?: boolean;
  result?: RechercheErgebnis;
  steps?: readonly RechercheSchritt[];
  onFollowUp?: (question: string, onlyInTheseSources: boolean) => void;
  onAction?: (action: RechercheAktion) => void;
  headingLevel?: 2 | 3 | 4;
  /** Angebotene Modi; mit höchstens einem Eintrag entfällt die Auswahl ganz (Standard: alle). */
  modes?: readonly RechercheModus[];
  /** Angebotene Quellenumfänge; mit höchstens einem Eintrag entfällt die Auswahl ganz (Standard: alle). */
  scopes?: readonly QuellenUmfang[];
  /** Aktionen unter der Antwort; leer blendet die Zeile aus (Standard: alle). */
  actions?: readonly RechercheAktion[];
  /** „Nur in diesen Quellen weiterfragen“ anbieten (Standard ja). */
  onlyHereOption?: boolean;
  labels?: Partial<RechercheAnsichtLabels>;
  className?: string;
}

const MODI: readonly RechercheModus[] = ["quick", "thorough", "deep"];
const UMFAENGE: readonly QuellenUmfang[] = ["web", "project", "both"];

function istSchwach(teil: AntwortTeil, quellen: ReadonlyMap<number, RechercheQuelle>) {
  if (teil.weak !== undefined) return teil.weak;
  if (teil.unsupported || teil.citations.length === 0) return false;
  return teil.citations.every((i) => {
    const art = quellen.get(i)?.sourceType;
    return art === "forum" || art === "unknown";
  });
}

export function RechercheAnsicht({
  projectTitle,
  question: frageProp,
  onQuestionChange,
  mode: modusProp,
  onModeChange,
  scope: umfangProp,
  onScopeChange,
  webEnabled = true,
  effort,
  onSubmit,
  busy = false,
  result,
  steps = [],
  onFollowUp,
  onAction,
  headingLevel = 3,
  modes = MODI,
  scopes = UMFAENGE,
  actions,
  onlyHereOption = true,
  labels: overrides,
  className,
}: RechercheAnsichtProps) {
  const L = useMemo(() => ({ ...RECHERCHE_LABELS, ...overrides }), [overrides]);
  const ids = useId();
  const [frageEigen, setFrageEigen] = useState(frageProp ?? "");
  const [modusEigen, setModusEigen] = useState<RechercheModus>(
    modes.includes("thorough") ? "thorough" : (modes[0] ?? "quick"),
  );
  const [umfangEigen, setUmfangEigen] = useState<QuellenUmfang>(
    scopes.includes(webEnabled ? "both" : "project")
      ? webEnabled
        ? "both"
        : "project"
      : (scopes[0] ?? "web"),
  );
  const [nurHier, setNurHier] = useState(false);
  const [hervor, setHervor] = useState<number | undefined>();
  const [ansage, setAnsage] = useState("");
  const ansageZaehler = useRef(0);
  const quellenRefs = useRef(new Map<number, HTMLLIElement>());

  const frage = frageProp ?? frageEigen;
  const modus = modusProp ?? modusEigen;
  const umfang = umfangProp ?? umfangEigen;
  const quellen = useMemo(
    () => new Map((result?.sources ?? []).map((q) => [q.index, q] as const)),
    [result],
  );

  const setFrage = (v: string) => {
    setFrageEigen(v);
    onQuestionChange?.(v);
  };
  const setModus = (v: RechercheModus) => {
    setModusEigen(v);
    onModeChange?.(v);
  };
  const setUmfang = (v: QuellenUmfang) => {
    setUmfangEigen(v);
    onScopeChange?.(v);
  };

  const zeigeQuelle = (n: number) => {
    setHervor(n);
    const q = quellen.get(n);
    if (q) {
      // Zero-Width-Space im Wechsel: dieselbe Ansage wird erneut vorgelesen.
      ansageZaehler.current += 1;
      setAnsage(`${L.highlighted(n, q.title)}${ansageZaehler.current % 2 ? "" : "\u200b"}`);
    }
    quellenRefs.current.get(n)?.scrollIntoView?.({ block: "nearest" });
  };

  const gesperrt = (u: QuellenUmfang) => !webEnabled && u !== "project";
  const senden = () => {
    if (busy || !frage.trim()) return;
    onSubmit?.({ question: frage.trim(), mode: modus, scope: umfang });
  };

  return (
    <div className={cx("wai-recherche", className)}>
      <div className="wai-recherche__raster">
        <div className="wai-recherche__haupt">
          <form
            className="wai-recherche__frage"
            onSubmit={(e) => {
              e.preventDefault();
              senden();
            }}
          >
            <label htmlFor={`${ids}-q`}>
              <b>{L.question}</b>
              {projectTitle && (
                <small className="wai-recherche__hinweis"> {L.inProject(projectTitle)}</small>
              )}
            </label>
            <textarea
              id={`${ids}-q`}
              className="wai-recherche__eingabe"
              value={frage}
              onChange={(e) => setFrage(e.target.value)}
              rows={2}
            />
            <div className="wai-recherche__zeile">
              {modes.length > 1 && (
                <Radiogruppe
                  name={`${ids}-modus`}
                  label={L.mode}
                  wert={modus}
                  onWert={setModus}
                  optionen={modes.map((m) => ({ wert: m, text: L.modes[m] }))}
                />
              )}
              {scopes.length > 1 && (
                <Radiogruppe
                  name={`${ids}-umfang`}
                  label={L.scope}
                  wert={umfang}
                  onWert={setUmfang}
                  optionen={scopes.map((u) => ({
                    wert: u,
                    text: L.scopes[u],
                    gesperrt: gesperrt(u),
                  }))}
                />
              )}
              {effort && <span className="wai-recherche__aufwand">{L.effort(effort)}</span>}
              <Button
                type="submit"
                variant="primary"
                className="wai-recherche__senden"
                isPending={busy}
                isDisabled={!frage.trim()}
              >
                {busy ? L.busy : L.submit}
              </Button>
            </div>
            {!webEnabled && <small className="wai-recherche__hinweis">{L.webLocked}</small>}
          </form>

          {result ? (
            <article className="wai-recherche__antwort" aria-label={L.answer}>
              {result.title &&
                (headingLevel === 2 ? (
                  <h2>{result.title}</h2>
                ) : headingLevel === 3 ? (
                  <h3>{result.title}</h3>
                ) : (
                  <h4>{result.title}</h4>
                ))}
              {result.answer.map((teil, i) => {
                const schwach = istSchwach(teil, quellen);
                const stil = cx(
                  teil.unsupported && "wai-recherche__ohne",
                  schwach && "wai-recherche__schwach",
                );
                const schnitt = teil.text.lastIndexOf(" ") + 1;
                const kopf = teil.text.slice(0, schnitt);
                const letztes = teil.text.slice(schnitt);
                return (
                  // biome-ignore lint/suspicious/noArrayIndexKey: Antwortteile sind eine feste Reihenfolge
                  <p key={i}>
                    <span className={stil}>{kopf}</span>
                    <span className="wai-recherche__nowrap">
                      <span className={stil}>{letztes}</span>
                      {teil.citations.map((n, k) => (
                        <sup key={`${k}-${n}`}>
                          <button
                            type="button"
                            className="wai-recherche__fn"
                            aria-label={L.footnote(n)}
                            aria-controls={`${ids}-s${n}`}
                            onClick={() => zeigeQuelle(n)}
                          >
                            {n}
                          </button>
                        </sup>
                      ))}
                    </span>
                    {teil.unsupported && (
                      <span className="wai-recherche__marke"> {L.noSource}</span>
                    )}
                    {schwach && <small className="wai-recherche__hinweis"> ({L.weakProof})</small>}
                  </p>
                );
              })}
              {result.contradictions?.map((w) => (
                <div
                  key={`${w.statement}-${w.sourceIndices.join("-")}`}
                  className="wai-recherche__widerspruch"
                >
                  <b>{L.contradiction}:</b> {w.statement}{" "}
                  <small>{L.contradictionRefs(w.sourceIndices)}</small>
                </div>
              ))}
              <p className="wai-recherche__kennz">
                <Icon name="sparkles" /> <b>{L.aiLabel}</b> · {result.provenance}
              </p>
              {(actions ?? (Object.keys(L.actions) as RechercheAktion[])).length > 0 && (
                <div className="wai-recherche__aktionen">
                  {(actions ?? (Object.keys(L.actions) as RechercheAktion[])).map((a) => (
                    <Button key={a} onPress={() => onAction?.(a)}>
                      {L.actions[a]}
                    </Button>
                  ))}
                </div>
              )}
              {result.followUps.length > 0 && (
                // biome-ignore lint/a11y/useSemanticElements: Gruppe ohne fieldset-Rahmen
                <div className="wai-recherche__folge" role="group" aria-label={L.followUp}>
                  <b>{L.followUp}</b>
                  {result.followUps.map((f, k) => (
                    <Button
                      key={`${k}-${f}`}
                      className="wai-recherche__folgefrage"
                      onPress={() => onFollowUp?.(f, nurHier)}
                    >
                      {f}
                    </Button>
                  ))}
                  {onlyHereOption && (
                    <label className="wai-recherche__nurhier">
                      <input
                        type="checkbox"
                        checked={nurHier}
                        onChange={(e) => setNurHier(e.target.checked)}
                      />{" "}
                      {L.onlyHere}
                    </label>
                  )}
                </div>
              )}
            </article>
          ) : (
            <p className="wai-recherche__leer">{L.noResult}</p>
          )}
        </div>
        {result && (
          <aside className="wai-recherche__seite" aria-label={L.sources(result.sources.length)}>
            <div className="wai-recherche__spalte">
              <div className="wai-recherche__spaltenkopf">{L.sources(result.sources.length)}</div>
              <ol className="wai-recherche__quellen">
                {result.sources.map((q) => (
                  <li
                    key={q.index}
                    id={`${ids}-s${q.index}`}
                    ref={(el) => {
                      if (el) quellenRefs.current.set(q.index, el);
                      else quellenRefs.current.delete(q.index);
                    }}
                    className="wai-recherche__quelle"
                    aria-current={hervor === q.index ? "true" : undefined}
                    value={q.index}
                  >
                    <span className="wai-recherche__nr" aria-hidden="true">
                      {q.index}
                    </span>
                    <div>
                      <b>{q.title}</b>
                      <small>
                        {q.host} · {L.sourceTypes[q.sourceType]} · {q.retrievedLabel}
                      </small>
                      <q>{q.excerpt}</q>
                    </div>
                  </li>
                ))}
              </ol>
            </div>
            {steps.length > 0 && (
              <div className="wai-recherche__spalte">
                <div className="wai-recherche__spaltenkopf">{L.steps}</div>
                <ol className="wai-recherche__schritte">
                  {steps.map((s, k) => (
                    <li key={`${k}-${s.label}`} data-state={s.state}>
                      <Icon name={s.state === "done" ? "check" : "refresh"} /> {s.label}
                    </li>
                  ))}
                </ol>
              </div>
            )}
          </aside>
        )}
      </div>
      {/* biome-ignore lint/a11y/useSemanticElements: Statusansage für Screenreader */}
      <div className="wai-recherche__nur-sr" role="status" aria-live="polite">
        {ansage}
      </div>
    </div>
  );
}

interface RadioProps<T extends string> {
  name: string;
  label: string;
  wert: T;
  onWert: (v: T) => void;
  optionen: readonly { wert: T; text: string; gesperrt?: boolean }[];
}

function Radiogruppe<T extends string>({ name, label, wert, onWert, optionen }: RadioProps<T>) {
  return (
    <fieldset className="wai-recherche__radios">
      <legend className="wai-recherche__nur-sr">{label}</legend>
      {optionen.map((o) => (
        <label key={o.wert} className="wai-recherche__radio">
          <input
            type="radio"
            name={name}
            value={o.wert}
            checked={wert === o.wert}
            disabled={o.gesperrt}
            onChange={() => onWert(o.wert)}
          />
          <span>{o.text}</span>
        </label>
      ))}
    </fieldset>
  );
}
