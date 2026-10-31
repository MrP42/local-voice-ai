import type { SprachKopplung, SprachZugang } from "@/bindings";

export type KopplungWarnung =
  "nonLatin" | "mixedScripts" | "duplicateName" | "duplicateClient";

const LETTER = /\p{L}/u;
const LATIN_LETTER = /\p{Script=Latin}/u;

const normalisiert = (text: string): string =>
  text.normalize("NFKC").trim().toLocaleLowerCase();

/**
 * Hinweise zu einer Kopplungsanfrage. Der Name ist frei gewählt und kommt von der anfragenden
 * App: Buchstaben fremder Schriften, die lateinischen zum Verwechseln ähneln (kyrillisches „С“
 * statt „C“), oder ein Name, den schon eine gekoppelte App trägt, können eine bekannte App
 * vortäuschen. Die Hinweise sagen es dem Nutzer; entscheiden muss er selbst.
 */
export const kopplungsWarnungen = (
  anfrage: Pick<SprachKopplung, "id" | "anzeige" | "client">,
  zugaenge: readonly SprachZugang[],
  weitere: readonly SprachKopplung[] = [],
): KopplungWarnung[] => {
  const warnungen: KopplungWarnung[] = [];
  const buchstaben = Array.from(anfrage.anzeige).filter((c) => LETTER.test(c));
  const fremd = buchstaben.some((c) => !LATIN_LETTER.test(c));
  if (fremd) {
    warnungen.push(
      buchstaben.some((c) => LATIN_LETTER.test(c))
        ? "mixedScripts"
        : "nonLatin",
    );
  }
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
