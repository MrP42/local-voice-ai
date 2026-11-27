import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Plus, Search, Sparkles, Trash2, Volume2 } from "lucide-react";
import { toast } from "sonner";
import { commands, type LexikonEintrag, type LexikonZiel } from "@/bindings";
import {
  befunde,
  dokumentFuerVorschlaege,
  gleich,
  hatFehler,
  ohneLeerzeilen,
  vorschlaegeUebernehmen,
  type LexikonBefund,
} from "@/lib/reader/lexikon";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Input } from "../../ui/Input";
import { TabList } from "../../ui/TabList";

type Bereich = "dokument" | "global";

interface AusspracheDialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  /** Buch der Bibliothek, dem die aktive Seite gehört. */
  buchId: string | null;
  /** Aktive Seite (ohne Buch: Regeln gelten nur für diese Seite). */
  seiteId: string | null;
  /** Text des Dokuments für die KI-Vorschläge (nur eine Wortliste geht hinaus). */
  dokumentText: string;
  /** Vorbelegtes Wort, z. B. die Markierung im Reader. */
  startWort?: string;
}

const LEER: LexikonEintrag = { wort: "", aussprache: "", gross_klein: false };
const SUCHE_AB = 8;

function zielFuer(
  bereich: Bereich,
  buchId: string | null,
  seiteId: string | null,
): LexikonZiel | null {
  if (bereich === "global") return { art: "global" };
  if (buchId) return { art: "buch", id: buchId };
  if (seiteId) return { art: "seite", id: seiteId };
  return null;
}

async function abspielen(text: string): Promise<void> {
  const res = await commands.ttsWortProbe(text);
  if (res.status !== "ok") throw new Error(res.error);
  const url = URL.createObjectURL(
    new Blob([new Uint8Array(res.data)], { type: "audio/wav" }),
  );
  const audio = new Audio(url);
  audio.onended = () => URL.revokeObjectURL(url);
  audio.onerror = () => URL.revokeObjectURL(url);
  await audio.play();
}

/**
 * Aussprache-Lexikon (lva#943): Regeln, wie die Stimme einzelne Wörter spricht, für dieses
 * Buch bzw. diese Seite oder für alle Texte. Phonon kennt keine Lautschrift; eine Regel
 * ersetzt das Wort beim Vorlesen durch eine Umschreibung in deutscher Schreibweise
 * („Microsoft“ → „Maikrosoft“). Der Text auf dem Bildschirm bleibt unverändert.
 *
 * Gespeichert wird nur auf „Speichern“, je Bereich die ganze Liste. KI-Vorschläge werden
 * nie gespeichert, bevor sie einzeln übernommen und dann gespeichert sind.
 */
export const AusspracheDialog: React.FC<AusspracheDialogProps> = ({
  open,
  onOpenChange,
  buchId,
  seiteId,
  dokumentText,
  startWort,
}) => {
  const { t } = useTranslation();
  const hatDokument = !!(buchId || seiteId);
  const [bereich, setBereich] = useState<Bereich>(
    hatDokument ? "dokument" : "global",
  );
  const [gespeichert, setGespeichert] = useState<
    Partial<Record<Bereich, LexikonEintrag[]>>
  >({});
  const [entwurf, setEntwurf] = useState<
    Partial<Record<Bereich, LexikonEintrag[]>>
  >({});
  const [laden, setLaden] = useState(false);
  const [ladeFehler, setLadeFehler] = useState<string | null>(null);
  const [speichern, setSpeichern] = useState(false);
  const [suche, setSuche] = useState("");
  const [spielt, setSpielt] = useState<string | null>(null);
  const [vorschlagLauf, setVorschlagLauf] = useState(false);
  const [vorschlaege, setVorschlaege] = useState<LexikonEintrag[] | null>(null);
  const [gewaehlt, setGewaehlt] = useState<Set<number>>(new Set());
  const [ohneWoerterbuch, setOhneWoerterbuch] = useState(false);
  const [beruehrt, setBeruehrt] = useState<Set<string>>(new Set());
  const neuesFeld = useRef<HTMLInputElement | null>(null);
  // Zählt Vorschlagsläufe; Schließen oder ein anderes Dokument macht laufende ungültig.
  const vorschlagLaufNr = useRef(0);
  const neueAussprache = useRef<HTMLInputElement | null>(null);
  const [fokusNeu, setFokusNeu] = useState<"wort" | "aussprache" | null>(null);

  // Beim Öffnen: beide Listen frisch laden, Entwürfe verwerfen.
  useEffect(() => {
    vorschlagLaufNr.current += 1;
    setVorschlagLauf(false);
    if (!open) return;
    let lebt = true;
    setBereich(hatDokument ? "dokument" : "global");
    setSuche("");
    setVorschlaege(null);
    setGewaehlt(new Set());
    setBeruehrt(new Set());
    setLadeFehler(null);
    setLaden(true);
    const bereiche: Bereich[] = hatDokument
      ? ["dokument", "global"]
      : ["global"];
    void Promise.all(
      bereiche.map(async (b) => {
        const ziel = zielFuer(b, buchId, seiteId);
        if (!ziel) return [b, []] as const;
        const r = await commands.lexikonLaden(ziel);
        if (r.status !== "ok") throw new Error(r.error);
        return [b, r.data] as const;
      }),
    )
      .then((paare) => {
        if (!lebt) return;
        const geladen = Object.fromEntries(paare) as Partial<
          Record<Bereich, LexikonEintrag[]>
        >;
        setGespeichert(geladen);
        const start: Partial<Record<Bereich, LexikonEintrag[]>> = {};
        for (const [b, liste] of paare) start[b] = liste.map((e) => ({ ...e }));
        if (startWort) {
          const b: Bereich = hatDokument ? "dokument" : "global";
          const liste = start[b] ?? [];
          if (
            !liste.some((e) => e.wort.toLowerCase() === startWort.toLowerCase())
          )
            start[b] = [...liste, { ...LEER, wort: startWort }];
          setFokusNeu("aussprache");
        }
        setEntwurf(start);
      })
      .catch((e: unknown) => {
        if (lebt) setLadeFehler(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (lebt) setLaden(false);
      });
    return () => {
      lebt = false;
    };
  }, [open, buchId, seiteId, hatDokument, startWort]);

  useEffect(() => {
    const ziel =
      fokusNeu === "wort"
        ? neuesFeld.current
        : fokusNeu === "aussprache"
          ? neueAussprache.current
          : null;
    if (ziel) {
      ziel.focus();
      setFokusNeu(null);
    }
  });

  const liste = entwurf[bereich] ?? [];
  const befundListe = useMemo(() => befunde(liste), [liste]);
  const geaendert = (b: Bereich) =>
    !gleich(ohneLeerzeilen(entwurf[b] ?? []), gespeichert[b] ?? []);
  const irgendwasGeaendert = geaendert("dokument") || geaendert("global");
  const fehlerIrgendwo = (["dokument", "global"] as Bereich[]).some((b) =>
    hatFehler(befunde(ohneLeerzeilen(entwurf[b] ?? []))),
  );

  const setzeListe = (neu: LexikonEintrag[]) =>
    setEntwurf((alt) => ({ ...alt, [bereich]: neu }));
  const aendern = (i: number, teil: Partial<LexikonEintrag>) =>
    setzeListe(liste.map((e, k) => (k === i ? { ...e, ...teil } : e)));
  const entfernen = (i: number) => setzeListe(liste.filter((_, k) => k !== i));
  const hinzu = () => {
    setSuche("");
    setzeListe([...liste, { ...LEER }]);
    setFokusNeu("wort");
  };

  const probe = useCallback(
    async (schluessel: string, text: string) => {
      if (!text.trim() || spielt) return;
      setSpielt(schluessel);
      try {
        await abspielen(text.trim());
      } catch (e) {
        toast.error(t("tts.lexikon.probeFehler"), {
          description: e instanceof Error ? e.message : String(e),
        });
      } finally {
        setSpielt(null);
      }
    },
    [spielt, t],
  );

  const vorschlagen = async () => {
    if (!dokumentText.trim() || vorschlagLauf) return;
    const lauf = ++vorschlagLaufNr.current;
    const gilt = () => vorschlagLaufNr.current === lauf;
    setVorschlagLauf(true);
    setVorschlaege(null);
    try {
      const r = await commands.lexikonVorschlaege(
        dokumentFuerVorschlaege(dokumentText),
      );
      if (!gilt()) return;
      if (r.status !== "ok") throw new Error(r.error);
      const bekannt = new Set(
        [...(entwurf.dokument ?? []), ...(entwurf.global ?? [])].map((e) =>
          e.wort.toLowerCase(),
        ),
      );
      const neu = r.data.vorschlaege.filter(
        (v) => !bekannt.has(v.wort.toLowerCase()),
      );
      setVorschlaege(neu);
      // Jeder Vorschlag wird einzeln gewählt; nichts ist vorausgewählt (Codex #984).
      setGewaehlt(new Set());
      setOhneWoerterbuch(!r.data.woerterbuch_verfuegbar);
    } catch (e) {
      if (!gilt()) return;
      toast.error(t("tts.lexikon.vorschlagFehler"), {
        description: e instanceof Error ? e.message : String(e),
      });
    } finally {
      if (gilt()) setVorschlagLauf(false);
    }
  };

  const uebernehmen = () => {
    if (!vorschlaege) return;
    const neu = vorschlaege.filter((_, i) => gewaehlt.has(i));
    setEntwurf((alt) => ({
      ...alt,
      [bereich]: vorschlaegeUebernehmen(alt[bereich] ?? [], neu),
    }));
    setVorschlaege(null);
    setGewaehlt(new Set());
    if (neu.length > 0)
      toast(t("tts.lexikon.uebernommen", { count: neu.length }));
  };

  const allesSpeichern = async () => {
    if (fehlerIrgendwo || speichern) {
      // Fehler sichtbar machen, auch in noch nicht verlassenen Feldern.
      setBeruehrt(new Set(["*"]));
      return;
    }
    setSpeichern(true);
    try {
      for (const b of ["dokument", "global"] as Bereich[]) {
        if (!(b in entwurf) || !geaendert(b)) continue;
        const ziel = zielFuer(b, buchId, seiteId);
        if (!ziel) continue;
        const sauber = ohneLeerzeilen(entwurf[b] ?? []);
        const r = await commands.lexikonSpeichern(ziel, sauber);
        if (r.status !== "ok") throw new Error(r.error);
        setGespeichert((alt) => ({ ...alt, [b]: sauber }));
        setEntwurf((alt) => ({ ...alt, [b]: sauber.map((e) => ({ ...e })) }));
      }
      toast(t("tts.lexikon.gespeichert"));
      onOpenChange(false);
    } catch (e) {
      toast.error(t("tts.lexikon.speichernFehler"), {
        description: e instanceof Error ? e.message : String(e),
      });
    } finally {
      setSpeichern(false);
    }
  };

  const sichtbar = (i: number, feld: "w" | "a", befund: LexikonBefund | null) =>
    befund !== null &&
    (befund !== "wort_leer" && befund !== "aussprache_leer"
      ? true
      : beruehrt.has("*") || beruehrt.has(`${i}${feld}`));

  const filter = suche.trim().toLowerCase();
  const zeilen = liste
    .map((e, i) => ({ e, i }))
    .filter(
      ({ e }) =>
        !filter ||
        e.wort.toLowerCase().includes(filter) ||
        e.aussprache.toLowerCase().includes(filter),
    );

  const tabs = hatDokument
    ? [
        {
          id: "dokument" as Bereich,
          label: buchId
            ? t("tts.lexikon.bereich.buch")
            : t("tts.lexikon.bereich.seite"),
        },
        {
          id: "global" as Bereich,
          label: t("tts.lexikon.bereich.global"),
        },
      ]
    : [];

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("tts.lexikon.titel")}
      description={t("tts.lexikon.beschreibung")}
      closeLabel={t("common.close")}
      className="w-full max-w-2xl"
      contentFades={false}
      footer={
        <div className="flex w-full flex-wrap items-center gap-2">
          <Button
            className="min-h-[44px]"
            variant="primary"
            disabled={laden || speichern || !irgendwasGeaendert || !!ladeFehler}
            onClick={() => void allesSpeichern()}
            data-testid="lexikon-speichern"
          >
            {speichern
              ? t("tts.lexikon.speichert")
              : t("tts.lexikon.speichern")}
          </Button>
          <Button
            className="min-h-[44px]"
            variant="secondary"
            onClick={() => onOpenChange(false)}
            data-testid="lexikon-abbrechen"
          >
            {t("common.cancel")}
          </Button>
          {irgendwasGeaendert && !speichern && (
            <span className="text-xs text-text/60" role="status">
              {t("tts.lexikon.ungespeichert")}
            </span>
          )}
        </div>
      }
    >
      {/* Während des Speicherns gesperrt: sonst gingen Eingaben in dieser Zeit verloren,
          weil der Dialog danach mit dem gespeicherten Stand schließt (Codex #984). */}
      <fieldset
        className="m-0 flex min-w-0 flex-col gap-3 border-0 p-0"
        disabled={speichern}
        aria-busy={speichern || undefined}
        data-testid="lexikon-dialog"
      >
        {tabs.length > 0 && (
          <TabList
            tabs={tabs}
            value={bereich}
            onChange={(b) => {
              setBereich(b);
              setSuche("");
              setVorschlaege(null);
            }}
            ariaLabel={t("tts.lexikon.bereich.label")}
            idPrefix="lexikon-bereich"
            compact
          />
        )}
        <p className="text-xs text-text/60">
          {bereich === "global"
            ? t("tts.lexikon.hinweis.global")
            : buchId
              ? t("tts.lexikon.hinweis.buch")
              : t("tts.lexikon.hinweis.seite")}
        </p>

        {laden ? (
          <p
            className="flex items-center gap-2 text-sm text-text/70"
            role="status"
          >
            <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
            {t("tts.lexikon.laedt")}
          </p>
        ) : ladeFehler ? (
          <p
            className="text-sm text-red-500"
            role="alert"
            data-testid="lexikon-ladefehler"
          >
            {t("tts.lexikon.ladeFehler")} {ladeFehler}
          </p>
        ) : (
          <>
            <div className="flex flex-wrap items-center gap-2">
              {liste.length >= SUCHE_AB && (
                <label className="relative flex min-w-[12rem] flex-1 items-center">
                  <span className="sr-only">{t("tts.lexikon.suche")}</span>
                  <Search
                    className="pointer-events-none absolute start-2 h-4 w-4 text-text/50"
                    aria-hidden="true"
                  />
                  <Input
                    className="ps-8"
                    value={suche}
                    placeholder={t("tts.lexikon.suche")}
                    onChange={(e) => setSuche(e.target.value)}
                    data-testid="lexikon-suche"
                  />
                </label>
              )}
              <Button
                variant="secondary"
                size="sm"
                className="min-h-[36px]"
                icon={<Plus width={16} height={16} aria-hidden="true" />}
                onClick={hinzu}
                data-testid="lexikon-hinzu"
              >
                {t("tts.lexikon.hinzu")}
              </Button>
              {bereich === "dokument" && dokumentText.trim() !== "" && (
                <Button
                  variant="secondary"
                  size="sm"
                  className="min-h-[36px]"
                  icon={
                    vorschlagLauf ? (
                      <Loader2
                        width={16}
                        height={16}
                        className="animate-spin"
                        aria-hidden="true"
                      />
                    ) : (
                      <Sparkles width={16} height={16} aria-hidden="true" />
                    )
                  }
                  disabled={vorschlagLauf}
                  title={t("tts.lexikon.vorschlagHinweis")}
                  onClick={() => void vorschlagen()}
                  data-testid="lexikon-vorschlagen"
                >
                  {vorschlagLauf
                    ? t("tts.lexikon.vorschlagLaeuft")
                    : t("tts.lexikon.vorschlagen")}
                </Button>
              )}
            </div>

            {vorschlaege !== null && (
              <section
                className="rounded-md border border-logo-primary/40 bg-logo-primary/5 p-3"
                aria-label={t("tts.lexikon.vorschlaegeTitel")}
                data-testid="lexikon-vorschlaege"
              >
                <div className="mb-2 flex flex-wrap items-center gap-2">
                  <h3 className="text-sm font-medium">
                    {t("tts.lexikon.vorschlaegeTitel")}
                  </h3>
                  <span className="rounded bg-mid-gray/15 px-1.5 text-[11px] text-text/70">
                    {t("tts.lexikon.lokal")}
                  </span>
                  <span className="flex-1" />
                  {vorschlaege.length > 0 && (
                    <>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() =>
                          setGewaehlt(
                            gewaehlt.size === vorschlaege.length
                              ? new Set()
                              : new Set(vorschlaege.map((_, i) => i)),
                          )
                        }
                      >
                        {gewaehlt.size === vorschlaege.length
                          ? t("tts.lexikon.keine")
                          : t("tts.lexikon.alle")}
                      </Button>
                      <Button
                        variant="primary"
                        size="sm"
                        disabled={gewaehlt.size === 0}
                        onClick={uebernehmen}
                        data-testid="lexikon-uebernehmen"
                      >
                        {t("tts.lexikon.uebernehmen", { count: gewaehlt.size })}
                      </Button>
                    </>
                  )}
                </div>
                {vorschlaege.length === 0 ? (
                  <p className="text-sm text-text/70">
                    {t("tts.lexikon.keineVorschlaege")}
                  </p>
                ) : (
                  <ul className="flex max-h-56 flex-col gap-1 overflow-y-auto">
                    {vorschlaege.map((v, i) => (
                      <li
                        key={`${v.wort}-${i}`}
                        className="flex min-h-[40px] items-center gap-2"
                      >
                        <label className="flex min-w-0 flex-1 cursor-pointer items-center gap-2 text-sm">
                          <input
                            type="checkbox"
                            checked={gewaehlt.has(i)}
                            onChange={(e) => {
                              const neu = new Set(gewaehlt);
                              if (e.target.checked) neu.add(i);
                              else neu.delete(i);
                              setGewaehlt(neu);
                            }}
                            data-testid={`lexikon-vorschlag-${i}`}
                          />
                          <span className="truncate font-medium">{v.wort}</span>
                          <span aria-hidden="true" className="text-text/50">
                            →
                          </span>
                          <span className="sr-only">
                            {t("tts.lexikon.gesprochenAls")}
                          </span>
                          <span className="truncate">{v.aussprache}</span>
                        </label>
                        <ProbeKnopf
                          aktiv={spielt === `v${i}`}
                          gesperrt={spielt !== null}
                          label={t("tts.lexikon.probe", { wort: v.wort })}
                          onClick={() => void probe(`v${i}`, v.aussprache)}
                        />
                      </li>
                    ))}
                  </ul>
                )}
                <p className="mt-2 text-xs text-text/60">
                  {t("tts.lexikon.vorschlagDatenschutz")}
                  {ohneWoerterbuch && ` ${t("tts.lexikon.ohneWoerterbuch")}`}
                </p>
              </section>
            )}

            {liste.length === 0 ? (
              <div
                className="rounded-md border border-dashed border-mid-gray/40 p-4 text-sm text-text/70"
                data-testid="lexikon-leer"
              >
                <p>{t("tts.lexikon.leer")}</p>
                <p className="mt-1 text-xs text-text/60">
                  {t("tts.lexikon.beispiel")}
                </p>
              </div>
            ) : (
              <div
                role="table"
                aria-label={t("tts.lexikon.titel")}
                className="flex flex-col"
              >
                <div
                  role="row"
                  className="hidden gap-2 px-1 pb-1 text-xs font-medium text-text/60 sm:grid sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]"
                >
                  <span role="columnheader">{t("tts.lexikon.wort")}</span>
                  <span role="columnheader">{t("tts.lexikon.aussprache")}</span>
                  <span role="columnheader" className="sr-only">
                    {t("tts.lexikon.aktionen")}
                  </span>
                </div>
                <ul className="flex max-h-[45vh] flex-col gap-2 overflow-y-auto pe-1">
                  {zeilen.map(({ e, i }) => {
                    const b = befundListe[i];
                    const wFehler = sichtbar(i, "w", b.wort) ? b.wort : null;
                    const aFehler = sichtbar(i, "a", b.aussprache)
                      ? b.aussprache
                      : null;
                    const letzte = i === liste.length - 1;
                    return (
                      <li
                        key={i}
                        role="row"
                        className="grid grid-cols-1 gap-2 rounded-md border border-mid-gray/20 p-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] sm:border-0 sm:p-0"
                        data-testid={`lexikon-zeile-${i}`}
                      >
                        <div role="cell" className="flex flex-col gap-1">
                          <Input
                            ref={letzte ? neuesFeld : undefined}
                            value={e.wort}
                            aria-label={t("tts.lexikon.wortZeile", {
                              n: i + 1,
                            })}
                            placeholder={t("tts.lexikon.wortBeispiel")}
                            autoComplete="off"
                            spellCheck={false}
                            aria-invalid={wFehler ? true : undefined}
                            onChange={(ev) =>
                              aendern(i, { wort: ev.target.value })
                            }
                            onBlur={() =>
                              setBeruehrt((s) => new Set(s).add(`${i}w`))
                            }
                            data-testid={`lexikon-wort-${i}`}
                          />
                          {wFehler && (
                            <p className="text-xs text-red-500">
                              {t(`tts.lexikon.fehler.${wFehler}`)}
                            </p>
                          )}
                        </div>
                        <div role="cell" className="flex flex-col gap-1">
                          <Input
                            ref={letzte ? neueAussprache : undefined}
                            value={e.aussprache}
                            aria-label={t("tts.lexikon.ausspracheZeile", {
                              n: i + 1,
                            })}
                            placeholder={t("tts.lexikon.ausspracheBeispiel")}
                            autoComplete="off"
                            spellCheck={false}
                            aria-invalid={aFehler ? true : undefined}
                            onChange={(ev) =>
                              aendern(i, { aussprache: ev.target.value })
                            }
                            onBlur={() =>
                              setBeruehrt((s) => new Set(s).add(`${i}a`))
                            }
                            onKeyDown={(ev) => {
                              if (ev.key === "Enter" && letzte) {
                                ev.preventDefault();
                                hinzu();
                              }
                            }}
                            data-testid={`lexikon-aussprache-${i}`}
                          />
                          {aFehler && (
                            <p className="text-xs text-red-500">
                              {t(`tts.lexikon.fehler.${aFehler}`)}
                            </p>
                          )}
                        </div>
                        <div
                          role="cell"
                          className="flex items-center gap-1 sm:self-start"
                        >
                          <label
                            className="flex min-h-[36px] cursor-pointer items-center gap-1.5 rounded-md px-1.5 text-xs text-text/70 hover:bg-mid-gray/10"
                            title={t("tts.lexikon.grossKleinHinweis")}
                          >
                            <input
                              type="checkbox"
                              checked={e.gross_klein}
                              onChange={(ev) =>
                                aendern(i, { gross_klein: ev.target.checked })
                              }
                              data-testid={`lexikon-gross-klein-${i}`}
                            />
                            {t("tts.lexikon.grossKlein")}
                          </label>
                          <ProbeKnopf
                            aktiv={spielt === `z${i}`}
                            gesperrt={
                              spielt !== null ||
                              !e.aussprache.trim() ||
                              b.aussprache !== null
                            }
                            label={t("tts.lexikon.probe", {
                              wort: e.wort || "…",
                            })}
                            onClick={() => void probe(`z${i}`, e.aussprache)}
                          />
                          <Button
                            variant="ghost"
                            size="sm"
                            type="button"
                            icon={
                              <Trash2
                                width={16}
                                height={16}
                                aria-hidden="true"
                              />
                            }
                            aria-label={t("tts.lexikon.entfernen", {
                              wort: e.wort || i + 1,
                            })}
                            title={t("tts.lexikon.entfernen", {
                              wort: e.wort || i + 1,
                            })}
                            onClick={() => entfernen(i)}
                            data-testid={`lexikon-entfernen-${i}`}
                          />
                        </div>
                      </li>
                    );
                  })}
                  {zeilen.length === 0 && (
                    <li className="text-sm text-text/60">
                      {t("tts.lexikon.keinTreffer")}
                    </li>
                  )}
                </ul>
              </div>
            )}
          </>
        )}
      </fieldset>
    </Dialog>
  );
};

const ProbeKnopf: React.FC<{
  aktiv: boolean;
  gesperrt: boolean;
  label: string;
  onClick(): void;
}> = ({ aktiv, gesperrt, label, onClick }) => (
  <Button
    variant="ghost"
    size="sm"
    type="button"
    icon={
      aktiv ? (
        <Loader2
          width={16}
          height={16}
          className="animate-spin"
          aria-hidden="true"
        />
      ) : (
        <Volume2 width={16} height={16} aria-hidden="true" />
      )
    }
    disabledFocusable={gesperrt && !aktiv}
    aria-label={label}
    title={label}
    onClick={() => {
      if (!gesperrt) onClick();
    }}
  />
);
