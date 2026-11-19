import {
  commands,
  type Consent,
  type Einladung,
  type Gestaltung,
  type Hinweis,
  type LokaleZeit,
  type Plan,
  type PlanInput,
  type PlanLink,
  type Result,
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
  /** Übergabe an Outlook: vermerkt sie (Status `uebergeben`), anders als die Vorschau. */
  einladung: (id: string, gestaltung: Gestaltung | null): Promise<Einladung> =>
    ok(commands.besprechungsplanungEinladung(id, gestaltung)),
  verschiebungBestaetigen: (id: string): Promise<PlanLink> =>
    ok(commands.besprechungsplanungVerschiebungBestaetigen(id)),
  abgleichen: () => ok(commands.besprechungsplanungAbgleichen()),
  /** Regelprüfung ohne Speichern. */
  pruefen: (input: PlanInput, beginn: LokaleZeit | null): Promise<Hinweis[]> =>
    commands.besprechungsplanungPruefen(input, beginn),
  /** Vorschau des Einladungstexts ohne Speichern und ohne Übergabe. */
  vorschau: (input: PlanInput, id: string | null): Promise<Einladung> =>
    commands.besprechungsplanungVorschau(input, id, null),
};
