import "./styles";
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { Link2Off, RefreshCw, Trash2, X } from "lucide-react";
import { toast } from "sonner";
import {
  commands,
  type EintragSicht,
  type LibraryBook,
  type RechercheStatus,
  type SuiteProjekt,
} from "@/bindings";
import {
  auspacken,
  fehlerText,
  projektEintrag,
  projektKurz,
  rechercheErgebnis,
  verweisZiel,
} from "@/lib/projekte";
import {
  meldeProjekte,
  useErwartung,
  useProjekteMeldung,
  type HaltEnde,
} from "@/lib/projekteBus";
import { notifyMeetingsChanged } from "@/lib/meetingsBus";
import {
  Projektordner,
  RechercheAnsicht,
  type ProjektAktion,
  type RechercheErgebnis,
} from "../../vendor/wai-ui/projekt";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Input } from "../ui/Input";
import { Select } from "../ui/Select";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { ReadAloudButton } from "../settings/meetings/ReadAloudButton";
import { useOrdnerLabels, useRechercheLabels } from "./labels";
import { useRechercheStart } from "./useRechercheStart";
import { useVorlesenGesperrt } from "./vorlesen";

export interface OrdnerRef {
  id: string;
  name: string;
  projektId: string | null;
}

interface Props {
  ordner: OrdnerRef;
  onClose: () => void;
  /** Eine Besprechung des Projektordners in den Aufnahmen öffnen. */
  onOpenMeeting: (meetingId: string) => void;
}

const KLASSEN = ["PUBLIC", "INTERNAL", "PERSONAL", "SPECIAL", "RESTRICTED"];

/**
 * Suite-Projekt eines Besprechungsordners (ADR-0026 PR-7): Verknüpfung, Projektordner (nur dieses
 * Projekt) mit Besprechungen, Büchern und Recherchen, Web-Freigabe und der Bereich Recherche (Modus
 * Schnell, Quelle Web). Die Freigabe der Suchanfragen übernimmt der `FreigabeWaechter`.
 */
export const SuiteProjektSeite: React.FC<Props> = ({
  ordner,
  onClose,
  onOpenMeeting,
}) => {
  const { t } = useTranslation();
  const ordnerLabels = useOrdnerLabels();
  const rechercheLabels = useRechercheLabels();
  const { start, busy } = useRechercheStart();

  const [projekte, setProjekte] = useState<SuiteProjekt[]>([]);
  const [eintraege, setEintraege] = useState<EintragSicht[]>([]);
  const [status, setStatus] = useState<RechercheStatus | null>(null);
  const [fehler, setFehler] = useState<string | null>(null);
  const [wahl, setWahl] = useState<string | null>(null);
  const [neuerTitel, setNeuerTitel] = useState("");
  const [neueKlasse, setNeueKlasse] = useState("PERSONAL");
  const [rechercheId, setRechercheId] = useState<string | null>(null);
  const [ergebnis, setErgebnis] = useState<RechercheErgebnis | undefined>();
  const [vorlese, setVorlese] = useState("");
  const [ergebnisKlasse, setErgebnisKlasse] = useState<string | null>(null);
  const vorlesenGesperrt = useVorlesenGesperrt(ergebnisKlasse);
  const [meldung, setMeldung] = useState<string | null>(null);
  const [buecher, setBuecher] = useState<LibraryBook[]>([]);
  const [buchWahl, setBuchWahl] = useState<string | null>(null);
  const [loeschen, setLoeschen] = useState(false);
  const [adresse, setAdresse] = useState("");
  const rechercheRef = useRef<HTMLDivElement>(null);
  const ergebnisRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (ergebnis) ergebnisRef.current?.scrollIntoView({ block: "nearest" });
  }, [ergebnis]);

  const suite = projekte.find((p) => p.id === ordner.projektId) ?? null;

  const laden = useCallback(async () => {
    try {
      const liste = auspacken(await commands.projekteListe());
      setProjekte(liste);
      setStatus(auspacken(await commands.rechercheStatus()));
      if (ordner.projektId) {
        setEintraege(
          auspacken(await commands.projektEintraege(ordner.projektId)),
        );
      } else {
        setEintraege([]);
      }
      setFehler(null);
    } catch (e) {
      setFehler(fehlerText(e, t));
    }
  }, [ordner.projektId]);

  useEffect(() => {
    void laden();
  }, [laden]);
  useProjekteMeldung("projekte", () => void laden());

  useEffect(() => {
    commands
      .libraryList()
      .then((r) => setBuecher(auspacken(r)))
      .catch(() => setBuecher([]));
  }, []);

  const zeigeRecherche = useCallback(
    async (id: string) => {
      try {
        const d = auspacken(await commands.rechercheLesen(id));
        setMeldung(null);
        setRechercheId(d.id);
        setErgebnis(rechercheErgebnis(d, t));
        setErgebnisKlasse(d.dataClass);
        setVorlese(d.readAloud);
      } catch (e) {
        setMeldung(fehlerText(e, t));
      }
    },
    [t],
  );

  // Nur eine Recherche, auf die diese Seite wartet (Kennung des Halts bzw. des Laufs), erscheint hier.
  const erwartung = useErwartung(
    (id) => void zeigeRecherche(id),
    (_id, wie: HaltEnde) =>
      setMeldung(
        wie === "abgelehnt"
          ? t("projekte.freigabe.nichtsGesendet")
          : wie === "abgelaufen"
            ? t("projekte.freigabe.abgelaufen")
            : t("projekte.fehler.unbekannt"),
      ),
  );

  const aenderung = useCallback(async () => {
    notifyMeetingsChanged();
    meldeProjekte("projekte");
  }, []);

  const tun = async (f: () => Promise<void>) => {
    try {
      await f();
      setFehler(null);
    } catch (e) {
      setFehler(fehlerText(e, t));
    }
  };

  const verknuepfen = (suiteId: string) =>
    tun(async () => {
      const b = auspacken(
        await commands.projektOrdnerVerknuepfen(ordner.id, suiteId),
      );
      toast.success(t("projekte.seite.verknuepft", { count: b.zugeordnet }));
      if (b.uebersprungen > 0) {
        toast.info(
          t("projekte.seite.uebersprungen", { count: b.uebersprungen }),
        );
      }
      await aenderung();
    });

  const anlegen = () =>
    tun(async () => {
      const p = auspacken(
        await commands.projektAnlegen(neuerTitel, neueKlasse, false),
      );
      setNeuerTitel("");
      await verknuepfen(p.id);
    });

  const loesen = () =>
    tun(async () => {
      auspacken(await commands.projektOrdnerLoesen(ordner.id));
      setErgebnis(undefined);
      setRechercheId(null);
      await aenderung();
    });

  const abgleichen = () =>
    tun(async () => {
      if (!ordner.projektId) return;
      auspacken(await commands.projektAbgleichen(ordner.projektId));
      await aenderung();
    });

  const webSchalten = (an: boolean) =>
    tun(async () => {
      if (!suite) return;
      auspacken(await commands.projektWebFreigabe(suite.id, an));
      meldeProjekte("projekte");
    });

  const buchHinzu = () =>
    tun(async () => {
      if (!suite || !buchWahl) return;
      auspacken(await commands.projektBuchZuordnen(suite.id, buchWahl));
      setBuchWahl(null);
      meldeProjekte("projekte");
    });

  const projektLoeschen = () =>
    tun(async () => {
      if (!suite) return;
      const r = auspacken(await commands.projektLoeschen(suite.id));
      setLoeschen(false);
      toast.success(
        t("projekte.seite.geloescht", { count: r.recherchenGeloescht }),
      );
      setErgebnis(undefined);
      setRechercheId(null);
      setVorlese("");
      setErgebnisKlasse(null);
      erwartung.leeren();
      await aenderung();
    });

  const rechercheLoeschen = () =>
    tun(async () => {
      if (!rechercheId) return;
      auspacken(await commands.rechercheLoeschen(rechercheId));
      setErgebnis(undefined);
      setRechercheId(null);
      setVorlese("");
      setErgebnisKlasse(null);
      toast.success(t("projekte.seite.rechercheGeloescht"));
      await aenderung();
    });

  const adresseSpeichern = () =>
    tun(async () => {
      const s = auspacken(
        await commands.rechercheAdresseSetzen(adresse.trim() || null),
      );
      setStatus(s);
      setAdresse("");
    });

  const frageStellen = async (frage: string) => {
    if (!suite) return;
    setMeldung(null);
    setErgebnis(undefined);
    erwartung.leeren();
    const r = await start(frage, suite.id);
    if (r.status === "fertig" && r.rechercheId)
      void zeigeRecherche(r.rechercheId);
    else if (r.status === "freigabe" && r.rechercheId) {
      erwartung.erwarte(r.rechercheId);
      setMeldung(r.message ?? null);
    } else setMeldung(r.message ?? null);
  };

  const aktion = (a: ProjektAktion, eintragId?: string) => {
    if (a === "start-research") {
      rechercheRef.current?.scrollIntoView({ block: "start" });
      rechercheRef.current
        ?.querySelector<HTMLElement>("textarea, input")
        ?.focus();
      return;
    }
    if (a !== "open" || !eintragId) return;
    const ziel = verweisZiel(eintraege.find((e) => e.id === eintragId)?.link);
    if (ziel?.art === "besprechung") onOpenMeeting(ziel.id);
    else if (ziel?.art === "recherche") void zeigeRecherche(ziel.id);
  };

  const projektOptionen = useMemo(
    () =>
      projekte
        .filter((p) => !p.archived)
        .map((p) => ({ value: p.id, label: p.title })),
    [projekte],
  );
  const buchOptionen = useMemo(
    () => buecher.map((b) => ({ value: b.id, label: b.title })),
    [buecher],
  );
  const klassenOptionen = KLASSEN.map((k) => ({
    value: k,
    label: ordnerLabels.classes[k as keyof typeof ordnerLabels.classes],
  }));

  return (
    <section
      data-testid="suite-projekt"
      aria-label={t("projekte.seite.titel")}
      className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3"
    >
      <header className="flex flex-wrap items-center gap-2">
        <div className="min-w-[12rem] flex-1">
          <h2 className="break-words text-sm font-semibold">
            {t("projekte.seite.ordner", { name: ordner.name })}
          </h2>
          <p className="text-xs text-text/60">{t("projekte.seite.hinweis")}</p>
        </div>
        {suite && (
          <>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => void abgleichen()}
              icon={<RefreshCw width={14} height={14} aria-hidden="true" />}
            >
              {t("projekte.seite.abgleichen")}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => void loesen()}
              icon={<Link2Off width={14} height={14} aria-hidden="true" />}
              data-testid="suite-loesen"
            >
              {t("projekte.seite.loesen")}
            </Button>
          </>
        )}
        <Button
          size="sm"
          variant="ghost"
          aria-label={t("projekte.seite.schliessen")}
          onClick={onClose}
          icon={<X width={14} height={14} aria-hidden="true" />}
        />
      </header>

      {fehler && (
        <p
          role="alert"
          className="text-xs text-[var(--colorPaletteRedForeground1)]"
        >
          {fehler}
        </p>
      )}

      {!suite && (
        <div
          data-testid="suite-verknuepfen"
          className="space-y-3 rounded-lg border border-mid-gray/30 p-3"
        >
          <p className="text-sm">{t("projekte.seite.nichtVerknuepft")}</p>
          <div className="flex flex-wrap items-end gap-2">
            <div className="min-w-[14rem] flex-1">
              <label className="mb-1 block text-xs text-text/60">
                {t("projekte.seite.vorhanden")}
              </label>
              <Select
                value={wahl}
                options={projektOptionen}
                placeholder={t("projekte.seite.waehlen")}
                ariaLabel={t("projekte.seite.vorhanden")}
                onChange={setWahl}
              />
            </div>
            <Button
              size="md"
              variant="primary"
              disabled={!wahl}
              onClick={() => wahl && void verknuepfen(wahl)}
            >
              {t("projekte.seite.verknuepfen")}
            </Button>
          </div>
          <div className="flex flex-wrap items-end gap-2 border-t border-mid-gray/20 pt-3">
            <div className="min-w-[12rem] flex-1">
              <label className="mb-1 block text-xs text-text/60">
                {t("projekte.seite.neuerTitel")}
              </label>
              <Input
                value={neuerTitel}
                maxLength={200}
                aria-label={t("projekte.seite.neuerTitel")}
                onChange={(e) => setNeuerTitel(e.target.value)}
              />
            </div>
            <div className="w-48">
              <label className="mb-1 block text-xs text-text/60">
                {t("projekte.seite.klasse")}
              </label>
              <Select
                value={neueKlasse}
                options={klassenOptionen}
                ariaLabel={t("projekte.seite.klasse")}
                onChange={(v) => v && setNeueKlasse(v)}
              />
            </div>
            <Button
              size="md"
              disabled={neuerTitel.trim() === ""}
              onClick={() => void anlegen()}
            >
              {t("projekte.seite.anlegen")}
            </Button>
          </div>
        </div>
      )}

      {suite && (
        <>
          <div className="flex flex-wrap items-end gap-x-6 gap-y-2">
            <div className="min-w-[16rem] flex-1">
              <ToggleSwitch
                checked={suite.webResearchEnabled}
                onChange={(an) => void webSchalten(an)}
                label={t("projekte.seite.webLabel")}
                description={t("projekte.seite.webHinweis")}
                descriptionMode="inline"
                testId="suite-web"
              />
            </div>
            <div className="flex min-w-[14rem] items-end gap-2">
              <div className="w-56">
                <Select
                  value={buchWahl}
                  options={buchOptionen}
                  placeholder={t("projekte.seite.buchWaehlen")}
                  ariaLabel={t("projekte.seite.buchWaehlen")}
                  onChange={setBuchWahl}
                />
              </div>
              <Button
                size="md"
                disabled={!buchWahl}
                onClick={() => void buchHinzu()}
              >
                {t("projekte.seite.buchHinzu")}
              </Button>
            </div>
            <Button
              size="sm"
              variant="danger-ghost"
              onClick={() => setLoeschen(true)}
              icon={<Trash2 width={14} height={14} aria-hidden="true" />}
            >
              {t("projekte.seite.loeschen")}
            </Button>
          </div>

          <Projektordner
            singleProject
            projects={[projektKurz(suite)]}
            selectedProjectId={suite.id}
            entries={eintraege.map(projektEintrag)}
            actions={["start-research", "open"]}
            headingLevel={3}
            labels={ordnerLabels}
            onAction={aktion}
          />

          <div
            ref={rechercheRef}
            data-testid="suite-recherche"
            className="space-y-2"
          >
            {status && !status.available && (
              <div
                role="status"
                data-testid="recherche-nicht-eingerichtet"
                className="rounded-md border border-yellow-500/40 bg-yellow-500/10 p-2 text-xs"
              >
                <p>
                  {status.reasonCode
                    ? t(`projekte.code.${status.reasonCode}`, {
                        defaultValue: status.reason ?? "",
                      })
                    : status.reason}
                </p>
                {status.canConfigure && (
                  <div className="mt-2 flex items-center gap-2">
                    <Input
                      value={adresse}
                      placeholder="https://suche.example.org"
                      aria-label={t("projekte.seite.adresse")}
                      onChange={(e) => setAdresse(e.target.value)}
                    />
                    <Button
                      size="sm"
                      disabled={adresse.trim() === ""}
                      onClick={() => void adresseSpeichern()}
                    >
                      {t("projekte.seite.adresseSpeichern")}
                    </Button>
                  </div>
                )}
              </div>
            )}
            {meldung && (
              <p
                role="status"
                data-testid="recherche-meldung"
                className="text-xs text-text/70"
              >
                {meldung}
              </p>
            )}
            <div ref={ergebnisRef} aria-live="polite">
              <RechercheAnsicht
                projectTitle={suite.title}
                headingLevel={3}
                modes={["quick"]}
                scopes={["web"]}
                actions={[]}
                onlyHereOption={false}
                webEnabled={suite.webResearchEnabled}
                busy={busy}
                result={ergebnis}
                labels={rechercheLabels}
                onSubmit={(q) => void frageStellen(q.question)}
                onFollowUp={(f) => void frageStellen(f)}
              />
            </div>
            {rechercheId && (
              <div className="flex items-center gap-2 text-xs">
                <ReadAloudButton
                  speakKey={`recherche:${rechercheId}`}
                  getMarkdown={() => vorlese}
                  testId="suite-readaloud"
                  disabled={vorlesenGesperrt}
                />
                {vorlesenGesperrt && (
                  <span className="text-text/60">
                    {t("projekte.vorlesen.nurLokal")}
                  </span>
                )}
                <Button
                  size="sm"
                  variant="danger-ghost"
                  data-testid="suite-recherche-loeschen"
                  onClick={() => void rechercheLoeschen()}
                  icon={<Trash2 width={14} height={14} aria-hidden="true" />}
                >
                  {t("projekte.seite.rechercheLoeschen")}
                </Button>
              </div>
            )}
          </div>
        </>
      )}

      <Dialog
        open={loeschen}
        onOpenChange={setLoeschen}
        title={t("projekte.seite.loeschenTitel")}
        closeLabel={t("projekte.seite.abbrechen")}
        footer={
          <>
            <Button variant="secondary" onClick={() => setLoeschen(false)}>
              {t("projekte.seite.abbrechen")}
            </Button>
            <Button variant="danger" onClick={() => void projektLoeschen()}>
              {t("projekte.seite.loeschen")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {t("projekte.seite.loeschenText", { name: suite?.title ?? "" })}
        </p>
      </Dialog>
    </section>
  );
};
