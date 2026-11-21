import {
  commands,
  type CalEvent,
  type Consent,
  type DocBasis,
  type Einladung,
  type Erreicht,
  type Gestaltung,
  type Hinweis,
  type LokaleZeit,
  type Meeting,
  type Plan,
  type PlanInput,
  type PlanLink,
  type ProtokollEntwurf,
  type ProtokollStand,
  type Result,
  type VorlageErgebnis,
} from "@/bindings";

/** Entpackt ein `Result` der Tauri-Befehle; der Fehlertext (`<code>` oder `<code>: <detail>`) wird geworfen. */
async function ok<T>(p: Promise<Result<T, string>>): Promise<T> {
  const r = await p;
  if (r.status === "ok") return r.data;
  throw new Error(r.error);
}

/** Befehle der Besprechungsplanung (lva#534). Inhalte der Planung gehen nie ins Log. */
export const api = {
  liste: () => ok(commands.besprechungsplanungListe()),
  anlegen: (input: PlanInput): Promise<Plan> =>
    ok(commands.besprechungsplanungAnlegen(input)),
  aendern: (id: string, input: PlanInput): Promise<Plan> =>
    ok(commands.besprechungsplanungAendern(id, input)),
  loeschen: (id: string) => ok(commands.besprechungsplanungLoeschen(id)),
  verknuepfung: (id: string): Promise<PlanLink | null> =>
    ok(commands.besprechungsplanungVerknuepfung(id)),
  einwilligungen: (id: string): Promise<Consent[]> =>
    ok(commands.besprechungsplanungEinwilligungen(id)),
  einwilligungSetzen: (einwilligung: Consent) =>
    ok(commands.besprechungsplanungEinwilligungSetzen(einwilligung)),
  einwilligungsVerlauf: (id: string) =>
    ok(commands.besprechungsplanungEinwilligungsVerlauf(id)),
  startModus: (id: string, anwesende: string[]) =>
    ok(commands.besprechungsplanungStartModus(id, anwesende)),
  anwesendeErgaenzen: (meetingId: string, id: string, personen: string[]) =>
    ok(commands.besprechungsplanungAnwesendeErgaenzen(meetingId, id, personen)),
  notizenStarten: (id: string, title: string): Promise<Meeting> =>
    ok(commands.besprechungsplanungNotizenStarten(id, title)),
  termine: (stunden: number): Promise<CalEvent[]> =>
    ok(commands.calendarUpcoming(stunden)),
  /**
   * Die Bestaetigung je Person steckt in den Einwilligungen; das Backend-Tor prueft beim Start
   * erneut und bleibt die letzte Instanz.
   */
  startAusTermin: (
    eventKey: string,
    captureSystem: boolean,
    targetMeetingId: string | null = null,
  ): Promise<Meeting> =>
    ok(
      commands.meetingsStartFromEvent(
        eventKey,
        null,
        true,
        captureSystem,
        null,
        "prompt",
        targetMeetingId,
      ),
    ),
  /** Übergabe an Outlook: vermerkt sie (Status `uebergeben`), anders als die Vorschau. */
  einladung: (id: string, gestaltung: Gestaltung | null): Promise<Einladung> =>
    ok(commands.besprechungsplanungEinladung(id, gestaltung)),
  verschiebungBestaetigen: (id: string): Promise<PlanLink> =>
    ok(commands.besprechungsplanungVerschiebungBestaetigen(id)),
  abgleichen: () => ok(commands.besprechungsplanungAbgleichen()),
  /** Stand des Protokolls der Planung (P5): Besprechung, Fassung, „Ziel erreicht“, Freigabe. */
  protokollStand: (id: string): Promise<ProtokollStand> =>
    ok(commands.besprechungsplanungProtokollStand(id)),
  /** Legt die Protokollvorlage aus der Agenda an und ordnet sie der Besprechung zu. */
  protokollVorlage: (id: string): Promise<VorlageErgebnis> =>
    ok(commands.besprechungsplanungProtokollvorlage(id)),
  /** Erzeugt das Protokoll auf dem vorhandenen Weg; das Einwilligungs-Tor gilt dort. */
  protokollErzeugen: (
    meetingId: string,
    templateId: string,
    basis: DocBasis | null,
  ) => ok(commands.meetingsGenerateMinutes(meetingId, templateId, basis)),
  /** „Ziel erreicht“, von Hand; `null` nimmt die Bewertung zurück. */
  erreichtSetzen: (
    id: string,
    erreicht: Erreicht | null,
  ): Promise<ProtokollStand> =>
    ok(commands.besprechungsplanungErreichtSetzen(id, erreicht)),
  protokollFreigeben: (id: string): Promise<ProtokollStand> =>
    ok(commands.besprechungsplanungProtokollFreigeben(id)),
  /** Entwurf aus dem freigegebenen Protokoll; versendet wird nie automatisch. */
  protokollEntwurf: (id: string): Promise<ProtokollEntwurf> =>
    ok(commands.besprechungsplanungProtokollEntwurf(id)),
  /** Regelprüfung ohne Speichern. */
  pruefen: (input: PlanInput, beginn: LokaleZeit | null): Promise<Hinweis[]> =>
    commands.besprechungsplanungPruefen(input, beginn),
  /** Vorschau des Einladungstexts ohne Speichern und ohne Übergabe. */
  vorschau: (input: PlanInput, id: string | null): Promise<Einladung> =>
    commands.besprechungsplanungVorschau(input, id, null),
};
