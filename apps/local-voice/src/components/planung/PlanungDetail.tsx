import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  Menu,
  MenuDivider,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  MessageBarTitle,
  tokens,
} from "@fluentui/react-components";
import {
  ArrowLeft,
  CalendarDays,
  Clock,
  Lock,
  Mic,
  MoreHorizontal,
  Play,
  Printer,
  Save,
  Send,
  Square,
  Trash2,
  Users,
  Video,
} from "lucide-react";
import type {
  CalEvent,
  Consent,
  Einladung,
  EinwilligungsEintrag,
  Hinweis,
  Plan,
  PlanLink,
  ProtokollStand,
} from "@/bindings";
import { flushAllNotes } from "../settings/meetings/notes/useNotesAutosave";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { api } from "./api";
import { Einwilligungen } from "./Einwilligungen";
import { EinwilligungsVerlauf } from "./EinwilligungsVerlauf";
import { AgendaZeitboxen } from "./AgendaZeitboxen";
import { EinladungsVorschau } from "./EinladungsVorschau";
import {
  beginnAusText,
  beginnTextAusMs,
  eingabeAusFormular,
  fehlerSchluessel,
  formularAusPlan,
  formularGueltig,
  istAbgeschlossen,
  leeresFormular,
  minutenAusText,
  statusMarken,
  uebergabeErlaubt,
  verschiebungOffen,
  type PlanForm,
} from "./logik";
import { PlanungFormular } from "./PlanungFormular";
import { ProtokollKarte } from "./ProtokollKarte";
import { Regelhinweise } from "./Regelhinweise";
import { LaufPanel } from "./LaufPanel";
import { StartPanel, type StartErgebnis } from "./StartPanel";
import { TerminKarte } from "./TerminKarte";
import {
  InselBefehl,
  PlanungInsel,
  registerPanelId,
  registerTabId,
  type InselFakt,
} from "./PlanungInsel";
import {
  registerInfos,
  startRegister,
  type LaufStand,
  type RegisterId,
} from "./register";
import { terminText } from "./Teile";
import { TerminfindungKarte } from "./terminfindung/TerminfindungKarte";
import { TerminfindungDialog } from "./terminfindung/TerminfindungDialog";

interface Props {
  /** Die Planung; `null` = neue, noch nicht gespeicherte. */
  plan: Plan | null;
  link: PlanLink | null;
  schmal: boolean;
  onZurueck: () => void;
  /** Nach Speichern, Übergabe, Bestätigung oder Löschen: Liste neu laden, ggf. `id` wählen. */
  onGeaendert: (id: string | null) => Promise<void>;
  /** Per Link geöffnete Terminfindung (lva#845); öffnet den Dialog einmal. */
  terminfindungId?: string | null;
}

/**
 * Wahr, solange gedruckt wird (Druckvorschau, `window.print()`): dann zeigt die Planung alle
 * Register und aufgeklappte Karten. Über das Attribut, weil die Grundregel für `[hidden]` in einer
 * Kaskadenebene mit `!important` steht und sich per CSS nicht überstimmen lässt.
 */
function useDruck(): boolean {
  const [druck, setDruck] = useState(false);
  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const abfrage = window.matchMedia("print");
    const folgen = () => setDruck(abfrage.matches);
    const an = () => setDruck(true);
    const aus = () => setDruck(abfrage.matches);
    abfrage.addEventListener("change", folgen);
    window.addEventListener("beforeprint", an);
    window.addEventListener("afterprint", aus);
    return () => {
      abfrage.removeEventListener("change", folgen);
      window.removeEventListener("beforeprint", an);
      window.removeEventListener("afterprint", aus);
    };
  }, []);
  return druck;
}

/** Stand für „geändert?“: der Beginn gehört nicht zur Planung (er stammt aus Outlook). */
const stand = (f: PlanForm): string => JSON.stringify({ ...f, beginn: "" });

/** Wartezeit nach der letzten Eingabe, bevor Regelprüfung und Vorschau neu fragen. */
const ENTPRELL_MS = 250;

/**
 * Das Blatt einer Planung (lva#844 U1): oben die Insel mit Titel, Marken, Eckdaten, Befehlen und
 * den Registern Planen · Termin & Zusagen · Durchführen · Protokoll; darunter je Register ein
 * Panel. Alle Panels bleiben gemountet und werden nur mit `hidden` ausgeblendet (R1): Lauf,
 * Zeitmessung und Einwilligungsverlauf leben über einen Registerwechsel hinweg.
 */
export const PlanungDetail: React.FC<Props> = ({
  plan: geladenerPlan,
  link,
  schmal,
  onZurueck,
  onGeaendert,
  terminfindungId = null,
}) => {
  const { t, i18n } = useTranslation();
  const druck = useDruck();
  const [tfOffen, setTfOffen] = useState(terminfindungId !== null);
  const [tfId, setTfId] = useState<string | null>(terminfindungId);
  // Die zurückgegebene Planung gilt sofort, auch solange die Liste noch lädt oder fehlschlägt.
  const [angelegterPlan, setAngelegterPlan] = useState<Plan | null>(null);
  const plan = geladenerPlan ?? angelegterPlan;
  const beginnAusTermin = link?.starts_at != null;
  // Nur beim Öffnen (die Seite hängt das Blatt je Auswahl neu ein, `key`).
  const [start] = useState<PlanForm>(() =>
    plan
      ? formularAusPlan(plan, beginnTextAusMs(link?.starts_at))
      : leeresFormular(),
  );
  const [lokalesFormular, setForm] = useState<PlanForm>(start);
  const kalenderBeginn = beginnAusTermin
    ? beginnTextAusMs(link?.starts_at)
    : null;
  const form = useMemo(
    () =>
      kalenderBeginn === null
        ? lokalesFormular
        : { ...lokalesFormular, beginn: kalenderBeginn },
    [lokalesFormular, kalenderBeginn],
  );
  const gespeichert = useRef(stand(start));
  const geaendert = stand(form) !== gespeichert.current;
  const [pruefung, setPruefung] = useState<{
    stand: string;
    hinweise: Hinweis[] | null;
    fehler: boolean;
  } | null>(null);
  const [vorschau, setVorschau] = useState<Einladung | null>(null);
  const [vorschauFehler, setVorschauFehler] = useState(false);
  const [einwilligungen, setEinwilligungen] = useState<Consent[] | null>(null);
  const [einwilligungenFehler, setEinwilligungenFehler] = useState(false);
  const [einwilligungenVersuch, setEinwilligungenVersuch] = useState(0);
  const [verlauf, setVerlauf] = useState<EinwilligungsEintrag[] | null>(null);
  const [verlaufFehler, setVerlaufFehler] = useState(false);
  const [verlaufVersuch, setVerlaufVersuch] = useState(0);
  const verlaufAbruf = useRef(0);
  const [termin, setTermin] = useState<CalEvent | null>(null);
  const [startOffen, setStartOffen] = useState(false);
  const [lauf, setLauf] = useState<StartErgebnis | null>(null);
  // Geprüfter Stand des laufenden Laufs (Art nach Widerruf, späte Personen), vom LaufPanel gemeldet.
  const [laufLive, setLaufLive] = useState<
    (LaufStand & { meetingId: string }) | null
  >(null);
  const [beendenOffen, setBeendenOffen] = useState(false);
  const [beendet, setBeendet] = useState(false);
  const beendenSperre = useRef(false);
  const [beendenFehler, setBeendenFehler] = useState<
    "notizen" | "andererLauf" | "startBusy" | "allgemein" | null
  >(null);
  const beendenAbbrechenRef = useRef<HTMLButtonElement>(null);
  // Stand des Protokolls, gemeldet von der (immer gemounteten) Protokollkarte.
  const [protokollStand, setProtokollStand] = useState<ProtokollStand | null>(
    null,
  );
  const [einwilligungSchreibt, setEinwilligungSchreibt] = useState(false);
  const [einwilligungFehler, setEinwilligungFehler] = useState(false);
  const einwilligungSperre = useRef(false);
  const einwilligungenAbruf = useRef(0);
  const [beschaeftigt, setBeschaeftigt] = useState(false);
  const speichert = useRef(false);
  const [loeschen, setLoeschen] = useState(false);
  // Beim Öffnen aus dem Datenstand abgeleitet; danach wählt nur die Nutzerin oder die Folge
  // einer eigenen Aktion (Start, Übergabe) das Register.
  const [register, setRegister] = useState<RegisterId>(() =>
    startRegister(plan, link, false),
  );
  const [meldungFokus, setMeldungFokus] = useState(0);
  const meldungRef = useRef<HTMLDivElement>(null);
  const terminPanelRef = useRef<HTMLDivElement>(null);
  const gesperrt = !!plan && istAbgeschlossen(plan.status);
  const planId = plan?.id ?? null;
  const pruefStand = JSON.stringify({ form, planId });
  // Schon im Rendern sperren: ein Effect allein käme nach der geänderten Eingabe zu spät.
  const hinweise = pruefung?.stand === pruefStand ? pruefung.hinweise : null;
  const pruefFehler = pruefung?.stand === pruefStand && pruefung.fehler;

  // Regelprüfung und Vorschau des Backends, kurz nach der letzten Eingabe.
  useEffect(() => {
    let veraltet = false;
    setPruefung(null);
    setVorschau(null);
    setVorschauFehler(false);
    const timer = setTimeout(() => {
      const eingabe = eingabeAusFormular(form);
      Promise.all([
        api.pruefen(eingabe, beginnAusText(form.beginn)),
        api.vorschau(eingabe, planId),
      ])
        .then(([h, v]) => {
          if (veraltet) return;
          setPruefung({ stand: pruefStand, hinweise: h, fehler: false });
          setVorschau(v);
          setVorschauFehler(false);
        })
        .catch(() => {
          if (veraltet) return;
          setPruefung({ stand: pruefStand, hinweise: null, fehler: true });
          setVorschau(null);
          setVorschauFehler(true);
        });
    }, ENTPRELL_MS);
    return () => {
      veraltet = true;
      clearTimeout(timer);
    };
  }, [form, planId, pruefStand]);

  // Stand der Einwilligungen der gespeicherten Planung.
  useEffect(() => {
    const abruf = ++einwilligungenAbruf.current;
    setEinwilligungen(null);
    setEinwilligungenFehler(false);
    if (!planId) return;
    let veraltet = false;
    api
      .einwilligungen(planId)
      .then((c) => {
        if (!veraltet && abruf === einwilligungenAbruf.current)
          setEinwilligungen(c);
      })
      .catch(() => {
        if (!veraltet && abruf === einwilligungenAbruf.current)
          setEinwilligungenFehler(true);
      });
    return () => {
      veraltet = true;
    };
  }, [planId, plan?.updated_at, einwilligungenVersuch]);

  // Der verknüpfte Termin liefert die Teilnehmenden und den Schlüssel für den Start.
  useEffect(() => {
    let veraltet = false;
    setTermin(null);
    if (!planId || !link) return;
    api
      .termine(24 * 60)
      .then((termine) => {
        if (veraltet) return;
        const gefunden = Array.isArray(termine)
          ? termine.find(
              (ev) =>
                !ev.cancelled &&
                (ev.uid === link.ical_uid ||
                  (link.remote_id != null && ev.uid === link.remote_id)) &&
                (link.starts_at == null || ev.starts_at === link.starts_at),
            )
          : null;
        setTermin(gefunden ?? null);
      })
      .catch(() => {
        if (!veraltet) setTermin(null);
      });
    return () => {
      veraltet = true;
    };
  }, [planId, link]);

  // Verlauf der Einwilligungen; späte Antworten dürfen den neuen Stand nicht ersetzen.
  useEffect(() => {
    const abruf = ++verlaufAbruf.current;
    setVerlauf(null);
    setVerlaufFehler(false);
    if (!planId) return;
    let veraltet = false;
    api
      .einwilligungsVerlauf(planId)
      .then((eintraege) => {
        if (!veraltet && abruf === verlaufAbruf.current) setVerlauf(eintraege);
      })
      .catch(() => {
        if (!veraltet && abruf === verlaufAbruf.current) setVerlaufFehler(true);
      });
    return () => {
      veraltet = true;
    };
  }, [planId, verlaufVersuch]);

  const neuLaden = () => {
    setEinwilligungenVersuch((v) => v + 1);
    setVerlaufVersuch((v) => v + 1);
  };

  // Nach dem Klick auf „Verschoben“: Fokus auf die Meldung (sonst auf das Panel).
  useEffect(() => {
    if (meldungFokus === 0) return;
    (meldungRef.current ?? terminPanelRef.current)?.focus();
  }, [meldungFokus]);

  const erlaubt =
    formularGueltig(form) && hinweise !== null && uebergabeErlaubt(hinweise);

  const einwilligungSetzen = async (
    person: string,
    state: Consent["state"],
  ) => {
    if (!planId || einwilligungSperre.current) return;
    einwilligungSperre.current = true;
    ++einwilligungenAbruf.current;
    setEinwilligungSchreibt(true);
    setEinwilligungFehler(false);
    try {
      await api.einwilligungSetzen({
        plan_id: planId,
        person,
        state,
        channel: "muendlich_bei_start",
        recorded_at: Date.now(),
      });
      setVerlaufVersuch((v) => v + 1);
      const abruf = ++einwilligungenAbruf.current;
      // Den nachgewiesenen Stand neu laden; keine optimistische Freigabe im Client.
      setEinwilligungen(null);
      setEinwilligungenFehler(false);
      try {
        const aktuell = await api.einwilligungen(planId);
        if (abruf === einwilligungenAbruf.current) setEinwilligungen(aktuell);
      } catch {
        if (abruf === einwilligungenAbruf.current)
          setEinwilligungenFehler(true);
      }
    } catch {
      setEinwilligungFehler(true);
    } finally {
      einwilligungSperre.current = false;
      setEinwilligungSchreibt(false);
    }
  };

  const melde = (e: unknown) => {
    const roh = e instanceof Error ? e.message : String(e);
    const code = roh.split(":")[0].trim();
    toast.error(t(fehlerSchluessel(roh), { code }));
  };

  const speichern = async () => {
    if (speichert.current || !erlaubt || gesperrt) return;
    speichert.current = true;
    setBeschaeftigt(true);
    try {
      const eingabe = eingabeAusFormular(form);
      const neu = plan
        ? await api.aendern(plan.id, eingabe)
        : await api.anlegen(eingabe);
      gespeichert.current = stand(form);
      if (!plan) setAngelegterPlan(neu);
      toast.success(t("planung.aktionen.gespeichert"));
      await onGeaendert(neu.id);
    } catch (e) {
      melde(e);
    } finally {
      speichert.current = false;
      setBeschaeftigt(false);
    }
  };

  const uebergeben = async () => {
    if (!plan) return;
    setBeschaeftigt(true);
    try {
      const e = await api.einladung(plan.id, null);
      try {
        await navigator.clipboard.write([
          new ClipboardItem({
            "text/html": new Blob([e.html], { type: "text/html" }),
            "text/plain": new Blob([e.text], { type: "text/plain" }),
          }),
        ]);
        toast.success(t("planung.aktionen.uebergeben"));
      } catch {
        toast.warning(t("planung.aktionen.kopierenFehler"));
      }
      await onGeaendert(plan.id);
      // Folge der eigenen Aktion: als Nächstes zählen Termin und Zusagen.
      setRegister("termin");
    } catch (e) {
      melde(e);
    } finally {
      setBeschaeftigt(false);
    }
  };

  const bestaetigen = async () => {
    if (!plan) return;
    setBeschaeftigt(true);
    try {
      await api.verschiebungBestaetigen(plan.id);
      toast.success(t("planung.termin.bestaetigt"));
      await onGeaendert(plan.id);
    } catch (e) {
      melde(e);
    } finally {
      setBeschaeftigt(false);
    }
  };

  const endgueltigLoeschen = async () => {
    if (!plan) return;
    setBeschaeftigt(true);
    try {
      await api.loeschen(plan.id);
      setLoeschen(false);
      toast.success(t("planung.aktionen.geloescht"));
      await onGeaendert(null);
    } catch (e) {
      melde(e);
    } finally {
      setBeschaeftigt(false);
    }
  };

  const uebergabeGesperrt =
    !plan || geaendert || !erlaubt || gesperrt || plan.status !== "entwurf";

  const laufStand: LaufStand | null = lauf
    ? laufLive?.meetingId === lauf.meetingId
      ? { art: laufLive.art, anwesende: laufLive.anwesende }
      : { art: lauf.art, anwesende: lauf.anwesende.length }
    : null;

  /**
   * „Besprechung beenden“: Die Aufnahme endet auf dem vorhandenen Weg der Besprechungen
   * (`meetings_stop`, vorher die Notizen sichern), aber nur, wenn die laufende Aufnahme zu diesem
   * Lauf gehört; die Zeitmessung endet mit dem Lauf (die Zeitboxen hängen am Lauf).
   */
  const beenden = async () => {
    if (!lauf || beendenSperre.current) return;
    // Sperre sofort (nicht erst nach dem nächsten Rendern): ein Doppelklick stoppt nur einmal.
    beendenSperre.current = true;
    setBeendet(true);
    setBeendenFehler(null);
    try {
      // Erst sichern, dann stoppen; ist nicht alles gesichert, wird nicht gestoppt.
      if (!(await flushAllNotes())) {
        setBeendenFehler("notizen");
        return;
      }
      if (lauf.art === "transkription")
        await api.aufnahmeBeenden(lauf.meetingId);
      setBeendenOffen(false);
      setLauf(null);
      setLaufLive(null);
      neuLaden();
      toast.success(t("planung.beendenDialog.beendet"));
    } catch (e) {
      const roh = e instanceof Error ? e.message : String(e);
      setBeendenFehler(
        roh === "anderer_lauf"
          ? "andererLauf"
          : roh === "start_busy"
            ? "startBusy"
            : "allgemein",
      );
    } finally {
      beendenSperre.current = false;
      setBeendet(false);
    }
  };
  const meldeLaufStand = useCallback(
    (stand: LaufStand & { meetingId: string }) => setLaufLive(stand),
    [],
  );
  const infos = registerInfos({
    plan,
    link,
    teilnehmende: termin?.attendees ?? null,
    agendaPunkte: form.agenda.filter((a) => a.topic.trim() !== "").length,
    regelFehler: !!hinweise?.some((h) => h.stufe === "fehler"),
    geaendert: !!plan && geaendert,
    lauf: laufStand,
    protokoll: plan ? protokollStand : null,
    sprache: i18n.language,
  });
  const registerGesperrt = useMemo(
    () =>
      new Set<RegisterId>(
        planId ? [] : ["termin", "durchfuehren", "protokoll"],
      ),
    [planId],
  );

  const titel =
    form.title.trim() ||
    (plan ? t("planung.liste.ohneTitel") : t("planung.neuTitel"));
  const marken = plan ? statusMarken(plan, link) : [];

  const fakten: InselFakt[] = [];
  const zeit = terminText(link, i18n.language);
  if (zeit) fakten.push({ id: "zeit", icon: CalendarDays, text: zeit });
  const dauer = minutenAusText(form.duration_min);
  if (dauer > 0)
    fakten.push({
      id: "dauer",
      icon: Clock,
      text: t("planung.liste.minuten", { count: dauer }),
    });
  if (termin && termin.attendees.length > 0)
    fakten.push({
      id: "teilnehmende",
      icon: Users,
      text: t("planung.insel.teilnehmende", { count: termin.attendees.length }),
    });
  if (link?.join_url)
    fakten.push({ id: "online", icon: Video, text: t("planung.insel.online") });
  if (form.classification.trim())
    fakten.push({
      id: "klassifizierung",
      icon: Lock,
      text: form.classification.trim(),
    });
  if (form.transcription_planned && !gesperrt && !lauf)
    fakten.push({
      id: "transkription",
      icon: Mic,
      text: t("planung.insel.transkriptionGeplant"),
    });

  const zuVerschiebung = () => {
    setRegister("termin");
    setMeldungFokus((n) => n + 1);
  };

  const aktionen = (
    <>
      {plan && lauf === null && (
        <InselBefehl text={t("planung.insel.starten")}>
          <Button
            type="button"
            size="md"
            // Primär nur im Register „Durchführen“; sonst trägt das Register seine eigene Primäre.
            variant={register === "durchfuehren" ? "primary" : "secondary"}
            icon={<Play size={16} aria-hidden="true" />}
            data-testid="planung-durchfuehren-start"
            disabled={gesperrt}
            onClick={() => setStartOffen(true)}
          >
            {t("planung.insel.starten")}
          </Button>
        </InselBefehl>
      )}
      {plan && lauf !== null && (
        <InselBefehl text={t("planung.insel.beenden")}>
          <Button
            type="button"
            size="md"
            icon={<Square size={16} aria-hidden="true" />}
            data-testid="planung-beenden"
            disabled={beendet}
            onClick={() => {
              setBeendenFehler(null);
              setBeendenOffen(true);
            }}
          >
            {t("planung.insel.beenden")}
          </Button>
        </InselBefehl>
      )}
      {register === "planen" && (
        <>
          <InselBefehl text={t("planung.aktionen.uebergebenKnopf")}>
            <Button
              type="button"
              size="md"
              icon={<Send size={16} aria-hidden="true" />}
              disabled={uebergabeGesperrt || beschaeftigt}
              title={t("planung.aktionen.uebergebenHilfe")}
              onClick={uebergeben}
              data-testid="planung-uebergeben"
            >
              {t("planung.aktionen.uebergebenKnopf")}
            </Button>
          </InselBefehl>
          <InselBefehl text={t("planung.aktionen.speichern")}>
            <Button
              type="button"
              variant="primary"
              size="md"
              icon={<Save size={16} aria-hidden="true" />}
              disabled={
                gesperrt || beschaeftigt || !erlaubt || (!!plan && !geaendert)
              }
              onClick={speichern}
              data-testid="planung-speichern"
            >
              {t("planung.aktionen.speichern")}
            </Button>
          </InselBefehl>
        </>
      )}
      {plan && (
        <Menu>
          <MenuTrigger disableButtonEnhancement>
            <Button
              type="button"
              size="md"
              icon={<MoreHorizontal size={16} aria-hidden="true" />}
              aria-label={t("planung.insel.mehr")}
              title={t("planung.insel.mehr")}
              data-testid="planung-mehr"
            />
          </MenuTrigger>
          <MenuPopover>
            <MenuList aria-label={t("planung.insel.mehr")}>
              <MenuItem
                icon={<Printer size={16} aria-hidden="true" />}
                onClick={() => window.print()}
              >
                {t("planung.insel.drucken")}
              </MenuItem>
              <MenuDivider />
              <MenuItem
                icon={<Trash2 size={16} aria-hidden="true" />}
                disabled={beschaeftigt}
                onClick={() => setLoeschen(true)}
                data-testid="planung-loeschen"
                style={{ color: tokens.colorPaletteRedForeground1 }}
              >
                {t("planung.insel.loeschen")}
              </MenuItem>
            </MenuList>
          </MenuPopover>
        </Menu>
      )}
    </>
  );

  const panel = (id: RegisterId) => ({
    role: "tabpanel" as const,
    id: registerPanelId(id),
    "aria-labelledby": registerTabId(id),
    hidden: register !== id && !druck,
    tabIndex: -1,
    className: `plg-panel plg-panel--${id}`,
    "data-testid": `planung-panel-${id}`,
  });

  return (
    <div className="plg-spalte plg-detail" data-testid="planung-detail">
      {schmal && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          icon={<ArrowLeft size={16} aria-hidden="true" />}
          onClick={onZurueck}
          className="plg-zurueck"
          data-testid="planung-zurueck"
        >
          {t("planung.aktionen.zurueck")}
        </Button>
      )}
      <PlanungInsel
        titel={titel}
        marken={marken}
        lauf={laufStand}
        fakten={fakten}
        aktionen={aktionen}
        register={register}
        infos={infos}
        gesperrt={registerGesperrt}
        onRegister={setRegister}
        onVerschoben={zuVerschiebung}
      />

      <div {...panel("planen")}>
        {gesperrt && (
          <p className="plg-meldung" role="status">
            {t("planung.aktionen.gesperrt")}
          </p>
        )}
        <div className="plg-detail__inhalt">
          <div className="plg-detail__form">
            <PlanungFormular
              form={form}
              onChange={setForm}
              gesperrt={gesperrt || beschaeftigt}
              beginnAusTermin={beginnAusTermin}
              teilnehmende={termin?.attendees}
            />
          </div>
          <aside
            className="plg-detail__seite"
            aria-label={t("planung.seitenleiste")}
          >
            <Regelhinweise
              hinweise={hinweise}
              form={form}
              fehler={pruefFehler}
            />
            <EinladungsVorschau einladung={vorschau} fehler={vorschauFehler} />
            <Einwilligungen
              einwilligungen={einwilligungen}
              transkriptionGeplant={form.transcription_planned}
              gespeichert={!!plan}
              fehler={einwilligungenFehler}
              onErneutVersuchen={() => setEinwilligungenVersuch((v) => v + 1)}
              onSetzen={plan ? einwilligungSetzen : undefined}
              beschaeftigt={einwilligungSchreibt}
              aufklappen={verlaufFehler}
              druck={druck}
            >
              {plan && (
                <EinwilligungsVerlauf
                  eintraege={verlauf}
                  fehler={verlaufFehler}
                  onErneutVersuchen={() => setVerlaufVersuch((v) => v + 1)}
                />
              )}
            </Einwilligungen>
            {einwilligungFehler && (
              <p className="plg-meldung plg-meldung--fehler" role="alert">
                {t("planung.einwilligungen.schreibFehler")}
              </p>
            )}
          </aside>
        </div>
      </div>

      <div {...panel("termin")} ref={terminPanelRef}>
        {plan && verschiebungOffen(link) && (
          <MessageBar
            ref={meldungRef}
            intent="warning"
            layout="multiline"
            tabIndex={-1}
            className="plg-meldungsleiste"
            data-testid="planung-verschiebung-meldung"
          >
            <MessageBarBody>
              <MessageBarTitle>
                {t("planung.termin.verschobenTitel")}
              </MessageBarTitle>
              {t("planung.termin.verschobenText")}
            </MessageBarBody>
            <MessageBarActions>
              <Button
                type="button"
                size="sm"
                variant="primary"
                disabled={beschaeftigt}
                onClick={bestaetigen}
                data-testid="planung-verschiebung-bestaetigen"
              >
                {t("planung.termin.bestaetigen")}
              </Button>
            </MessageBarActions>
          </MessageBar>
        )}
        {plan && <TerminKarte plan={plan} link={link} />}
        {plan && (
          <TerminfindungKarte
            planId={plan.id}
            pollId={tfId}
            gesperrt={gesperrt}
            onOeffnen={(id) => {
              setTfId(id);
              setTfOffen(true);
            }}
          />
        )}
      </div>

      <div {...panel("durchfuehren")}>
        {plan && (
          <section className="plg-karte" data-testid="planung-durchfuehren">
            <h2 className="plg-karte__titel">
              {t("planung.durchfuehren.titel")}
            </h2>
            {lauf === null ? (
              <>
                <p className="plg-karte__hilfe">
                  {t("planung.durchfuehren.hinweis")}{" "}
                  {t("planung.insel.startOben")}
                </p>
                <AgendaZeitboxen key={plan.id} agenda={plan.agenda} />
              </>
            ) : (
              <>
                <LaufPanel
                  key={lauf.meetingId}
                  plan={plan}
                  lauf={lauf}
                  einwilligungen={einwilligungen}
                  onEinwilligungGeaendert={neuLaden}
                  onStand={meldeLaufStand}
                />
                <Button
                  type="button"
                  variant="ghost"
                  data-testid="planung-durchfuehren-schliessen"
                  onClick={() => {
                    setLauf(null);
                    neuLaden();
                  }}
                >
                  {t("planung.durchfuehren.anzeigeSchliessen")}
                </Button>
              </>
            )}
          </section>
        )}
      </div>

      <div {...panel("protokoll")}>
        {plan && (
          <ProtokollKarte
            key={plan.id}
            plan={plan}
            onGeaendert={() => void onGeaendert(plan.id)}
            onStand={setProtokollStand}
          />
        )}
      </div>

      <Dialog
        open={startOffen && !!plan}
        onOpenChange={(offen) => {
          setStartOffen(offen);
          if (!offen) neuLaden();
        }}
        title={t("planung.beginnen.titel")}
        closeLabel={t("planung.beginnen.abbrechen")}
        className="max-w-2xl"
      >
        {plan && (
          <StartPanel
            key={`${plan.id}:${termin?.key ?? ""}`}
            plan={plan}
            eventKey={termin?.key ?? null}
            teilnehmende={termin?.attendees ?? []}
            onGestartet={(e) => {
              setLauf(e);
              setStartOffen(false);
              // Folge der eigenen Aktion: einmal nach „Durchführen“, kein späteres Zurückspringen.
              setRegister("durchfuehren");
              neuLaden();
              toast.success(
                t(
                  e.art === "transkription"
                    ? "planung.durchfuehren.gestartetMit"
                    : "planung.durchfuehren.gestartetNotizen",
                ),
              );
            }}
            onAbbrechen={() => {
              setStartOffen(false);
              neuLaden();
            }}
          />
        )}
      </Dialog>
      <Dialog
        open={beendenOffen && lauf !== null}
        onOpenChange={(offen) => {
          if (beendenSperre.current) return;
          setBeendenOffen(offen);
          if (!offen) setBeendenFehler(null);
        }}
        alert
        title={t("planung.beendenDialog.frage")}
        closeLabel={t("planung.beendenDialog.abbrechen")}
        initialFocusRef={beendenAbbrechenRef}
        footer={
          <>
            <Button
              type="button"
              variant="primary"
              disabled={beendet}
              onClick={() => void beenden()}
              data-testid="planung-beenden-ja"
            >
              {t("planung.beendenDialog.ja")}
            </Button>
            <Button
              ref={beendenAbbrechenRef}
              type="button"
              disabled={beendet}
              onClick={() => {
                setBeendenOffen(false);
                setBeendenFehler(null);
              }}
              data-testid="planung-beenden-abbrechen"
            >
              {t("planung.beendenDialog.abbrechen")}
            </Button>
          </>
        }
      >
        <p>
          {t(
            lauf?.art === "transkription"
              ? "planung.beendenDialog.textAufnahme"
              : "planung.beendenDialog.textNotizen",
          )}
        </p>
        {beendenFehler && (
          <MessageBar
            intent="error"
            layout="multiline"
            className="plg-meldungsleiste"
            data-testid="planung-beenden-fehler"
          >
            <MessageBarBody>
              {t(`planung.beendenDialog.fehler.${beendenFehler}`)}
            </MessageBarBody>
          </MessageBar>
        )}
      </Dialog>
      <Dialog
        open={loeschen}
        onOpenChange={setLoeschen}
        title={t("planung.loeschenDialog.frage", {
          titel: plan?.title.trim() || t("planung.liste.ohneTitel"),
        })}
        closeLabel={t("planung.loeschenDialog.abbrechen")}
        footer={
          <>
            <Button
              type="button"
              variant="danger"
              disabled={beschaeftigt}
              onClick={endgueltigLoeschen}
              data-testid="planung-loeschen-ja"
            >
              {t("planung.loeschenDialog.ja")}
            </Button>
            <Button type="button" onClick={() => setLoeschen(false)}>
              {t("planung.loeschenDialog.abbrechen")}
            </Button>
          </>
        }
      >
        <p>{t("planung.loeschenDialog.text")}</p>
      </Dialog>
      {plan && tfOffen && (
        <TerminfindungDialog
          open={tfOffen}
          onOpenChange={setTfOffen}
          pollId={tfId}
          planId={plan.id}
          titel={form.title}
          dauerMin={Number(form.duration_min) || plan.duration_min}
          personen={(termin?.attendees ?? [])
            .filter((a) => a.email && !a.is_self)
            .map((a) => ({ adresse: a.email ?? "", name: a.name ?? "" }))}
          onAngelegt={setTfId}
          outlookBestimmt={beginnAusTermin}
          onGewaehlt={(beginn, ende) => {
            setForm((f) => ({
              ...f,
              beginn: beginnTextAusMs(beginn),
              duration_min: String(Math.round((ende - beginn) / 60_000)),
            }));
            toast.success(t("planung.terminfindung.uebernommen"));
          }}
        />
      )}
    </div>
  );
};
