import {
  commands,
  type AntwortWert,
  type PersonAvailability,
  type PollDefinition,
  type PollView,
  type Rang,
  type Result,
  type ScheduleWindow,
} from "@/bindings";

async function ok<T>(p: Promise<Result<T, string>>): Promise<T> {
  const r = await p;
  if (r.status === "ok") return r.data;
  throw new Error(r.error);
}

/** Konto für Versand, Postfach und Verfügbarkeit: das erste eingeschaltete Microsoft-365-Konto. */
export interface Konto {
  id: string;
  adresse: string | null;
}

export async function m365Konto(): Promise<Konto | null> {
  const liste = await ok(commands.integrationsList());
  const v = liste.find(
    (e) => e.integration.kind === "m365" && e.integration.enabled,
  );
  if (!v) return null;
  let adresse: string | null = null;
  try {
    adresse = (await ok(commands.m365Status(v.integration.id))).account;
  } catch {
    adresse = null;
  }
  return { id: v.integration.id, adresse };
}

/** Befehle der Terminfindung (lva#845). Inhalte gehen nie ins Log. */
export const tfApi = {
  liste: (): Promise<PollDefinition[]> => ok(commands.pollList()),
  holen: (id: string): Promise<PollView> => ok(commands.pollGet(id)),
  anlegen: (kontoId: string, poll: PollDefinition): Promise<PollView> =>
    ok(commands.pollCreate(kontoId, poll)),
  aendern: (id: string, poll: PollDefinition): Promise<PollView> =>
    ok(commands.pollUpdate(id, poll)),
  verfuegbarkeit: (
    id: string,
    fenster: ScheduleWindow,
  ): Promise<PersonAvailability[]> =>
    ok(commands.pollAvailability(id, fenster)),
  senden: (id: string): Promise<PollView> => ok(commands.pollSend(id)),
  bestaetigen: (id: string, antwortId: number): Promise<PollView> =>
    ok(commands.pollAnswerConfirm(id, antwortId)),
  setzen: (
    id: string,
    person: string,
    werte: AntwortWert[],
  ): Promise<PollView> => ok(commands.pollAnswerSet(id, person, werte)),
  rang: (id: string): Promise<Rang[]> => ok(commands.pollRank(id)),
  schliessen: (id: string, entschieden: boolean): Promise<PollView> =>
    ok(commands.pollClose(id, entschieden)),
};
