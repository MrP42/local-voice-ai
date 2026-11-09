import {
  commands,
  type AblageArt,
  type AblageEinstellungen,
  type AblageKollisionswahl,
  type KiStufe,
  type Result,
} from "@/bindings";

/** Entpackt ein `Result` der Tauri-Befehle; der Fehlertext (`<code>: <text>`) wird als Error geworfen. */
async function ok<T>(p: Promise<Result<T, string>>): Promise<T> {
  const r = await p;
  if (r.status === "ok") return r.data;
  throw new Error(r.error);
}

export const api = {
  stand: () => ok(commands.ablageStand()),
  ordnerAnlegen: (eltern: string | null, name: string) =>
    ok(commands.ablageOrdnerAnlegen(eltern, name)),
  ordnerUmbenennen: (id: string, name: string) =>
    ok(commands.ablageOrdnerUmbenennen(id, name)),
  ordnerEinstellungen: (id: string, e: AblageEinstellungen) =>
    ok(commands.ablageOrdnerEinstellungen(id, e)),
  kiSetzen: (id: string, stufe: KiStufe | null) =>
    ok(commands.ablageKiSetzen(id, stufe)),
  freigabenNeuFestlegen: () => ok(commands.ablageFreigabenNeuFestlegen()),
  importieren: (
    ordner: string,
    pfad: string,
    wahl: AblageKollisionswahl | null,
  ) => ok(commands.ablageImportieren(ordner, pfad, wahl)),
  inPapierkorb: (art: AblageArt, id: string) =>
    ok(commands.ablageInPapierkorb(art, id)),
  wiederherstellen: (art: AblageArt, id: string) =>
    ok(commands.ablageWiederherstellen(art, id)),
  endgueltigLoeschen: (art: AblageArt, id: string) =>
    ok(commands.ablageEndgueltigLoeschen(art, id)),
  papierkorbLeeren: () => ok(commands.ablagePapierkorbLeeren()),
  fassungen: (datei: string) => ok(commands.ablageFassungen(datei)),
  dateiUmbenennen: (id: string, name: string) =>
    ok(commands.ablageDateiUmbenennen(id, name)),
  bestandNeuAufbauen: () => ok(commands.ablageBestandNeuAufbauen()),
  gefundeneAufnehmen: (datei: string, ordner: string) =>
    ok(commands.ablageGefundeneAufnehmen(datei, ordner)),
  suchen: (anfrage: string) => ok(commands.ablageSuchen(anfrage)),
  indexNeuAufbauen: () => ok(commands.ablageIndexNeuAufbauen()),
};
