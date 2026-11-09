import type { SprachKopplung, SprachZugang } from "@/bindings";

export type KopplungWarnung = "duplicateName" | "duplicateClient";

const normalisiert = (text: string): string =>
  text.normalize("NFKC").trim().toLocaleLowerCase();

/**
 * Hinweise zu einer Kopplungsanfrage. Der Name ist frei gewählt und kommt von der anfragenden
 * App: Ein Name, den schon eine gekoppelte App trägt, kann eine bekannte App vortäuschen.
 * (Der Dienst lässt im Namen nur lateinische Buchstaben, Umlaute und ß zu, Mischschrift
 * ist daher ausgeschlossen.) Die Hinweise sagen es dem Nutzer; entscheiden muss er selbst.
 */
export const kopplungsWarnungen = (
  anfrage: Pick<SprachKopplung, "id" | "anzeige" | "client">,
  zugaenge: readonly SprachZugang[],
  weitere: readonly SprachKopplung[] = [],
): KopplungWarnung[] => {
  const warnungen: KopplungWarnung[] = [];
  const name = normalisiert(anfrage.anzeige);
  const aktiv = zugaenge.filter((z) => z.widerrufen_ms == null);
  if (
    aktiv.some((z) => normalisiert(z.anzeige) === name) ||
    weitere.some((k) => k.id !== anfrage.id && normalisiert(k.anzeige) === name)
  ) {
    warnungen.push("duplicateName");
  }
  if (
    aktiv.some((z) => z.client === anfrage.client) ||
    weitere.some((k) => k.id !== anfrage.id && k.client === anfrage.client)
  ) {
    warnungen.push("duplicateClient");
  }
  return warnungen;
};
