import { test, expect } from "@playwright/test";
import {
  filtere,
  normalisiere,
  sprungZiel,
  trefferTeile,
} from "../src/components/ui/auswahlSuche";

// Auswahlfelder mit Buchstabensprung und Teilwortsuche (lva#839): reine Such- und
// Normalisierungslogik. Kein Browser, nur synthetische Daten (example.org).

const eintraege = (...labels: string[]) => labels.map((label) => ({ label }));
const labels = (liste: { eintrag: { label: string } }[]) =>
  liste.map((t) => t.eintrag.label);

test("Normalisierung: Groß/Klein, ä/ae, ö/oe, ü/ue, ß/ss und andere Akzente", () => {
  expect(normalisiere("Jörg MÜLLER")).toBe("joerg mueller");
  expect(normalisiere("Schäfer")).toBe(normalisiere("SCHAEFER"));
  expect(normalisiere("Straße")).toBe("strasse");
  expect(normalisiere("STRAẞE")).toBe("strasse");
  expect(normalisiere("öffentlich")).toBe("oeffentlich");
  expect(normalisiere("José")).toBe("jose");
  // Zerlegte Umlaute (a + Trema) zählen wie der fertige Buchstabe.
  expect(normalisiere("Mu\u0308ller")).toBe("mueller");
});

test("Teilwort an beliebiger Stelle, Anfangstreffer zuerst", () => {
  const liste = eintraege(
    "c@example.org",
    "Bernd Maier",
    "Jörg Müller",
    "Mara Klein",
    "Anna Schäfer",
  );
  // Leerer Text: alles, unverändert, ohne Hervorhebung.
  const alle = filtere(liste, "  ");
  expect(labels(alle)).toEqual(liste.map((e) => e.label));
  expect(alle.every((t) => t.bereich === null)).toBe(true);

  // Ein einzelner Buchstabe bleibt Filtertext: Feldanfang vor Wortanfang vor Wortmitte.
  expect(labels(filtere(liste, "m"))).toEqual([
    "Mara Klein",
    "Bernd Maier",
    "Jörg Müller",
    "c@example.org",
  ]);
  // Mitten im Wort, ohne Groß/Klein.
  expect(labels(filtere(liste, "LLER"))).toEqual(["Jörg Müller"]);
  expect(labels(filtere(liste, "chäf"))).toEqual(["Anna Schäfer"]);
  expect(labels(filtere(liste, "xyz"))).toEqual([]);
});

test("Umlaut-Toleranz in beide Richtungen", () => {
  const liste = eintraege(
    "Jörg Müller",
    "Anna Schäfer",
    "Petra Strauß",
    "Uwe Haas",
  );
  expect(labels(filtere(liste, "mueller"))).toEqual(["Jörg Müller"]);
  expect(labels(filtere(liste, "joerg"))).toEqual(["Jörg Müller"]);
  expect(labels(filtere(liste, "schaefer"))).toEqual(["Anna Schäfer"]);
  expect(labels(filtere(liste, "strauss"))).toEqual(["Petra Strauß"]);
  // Umgekehrt: der Umlaut im Suchtext findet die Umschreibung.
  expect(labels(filtere(liste, "haäs"))).toEqual([]);
  expect(labels(filtere(eintraege("Mueller GmbH"), "Mü"))).toEqual([
    "Mueller GmbH",
  ]);
});

test("Keine Teiltreffer, die nur aus einem expandierten Zeichen entstehen", () => {
  const baer = eintraege("Bär");
  // „e“ stammt aus „ä→ae“, „a“ nur aus der Expansion: kein Treffer.
  expect(labels(filtere(baer, "er"))).toEqual([]);
  expect(labels(filtere(baer, "a"))).toEqual([]);
  expect(labels(filtere(baer, "ba"))).toEqual([]);
  expect(labels(filtere(baer, "ear"))).toEqual([]);
  expect(labels(filtere(eintraege("Petra Strauß"), "aus"))).toEqual([]);
  expect(labels(filtere(eintraege("Jörg Müller"), "mu"))).toEqual([]);
  // Auch der zerlegte Umlaut (u + Trema) zählt als ein Zeichen.
  expect(labels(filtere(eintraege("Mu\u0308ller"), "mu"))).toEqual([]);
  expect(labels(filtere(eintraege("Mu\u0308ller"), "mue"))).toEqual([
    "Mu\u0308ller",
  ]);
  // Ein echter Treffer an anderer Stelle zählt.
  expect(labels(filtere(eintraege("Bär Hase"), "a"))).toEqual(["Bär Hase"]);
  const [hase] = filtere(eintraege("Bär Hase"), "a");
  expect(hase.bereich).toEqual([5, 6]);
  // Erlaubt bleibt die ganze Umschreibung, in beide Richtungen.
  expect(labels(filtere(baer, "baer"))).toEqual(["Bär"]);
  expect(labels(filtere(baer, "bä"))).toEqual(["Bär"]);
  expect(labels(filtere(eintraege("Baer"), "bär"))).toEqual(["Baer"]);
  expect(labels(filtere(eintraege("Jörg Müller"), "mue"))).toEqual([
    "Jörg Müller",
  ]);
  expect(labels(filtere(eintraege("Mueller"), "mü"))).toEqual(["Mueller"]);
  expect(labels(filtere(eintraege("Petra Strauß"), "strauss"))).toEqual([
    "Petra Strauß",
  ]);
  expect(labels(filtere(eintraege("Petra Strauss"), "strauß"))).toEqual([
    "Petra Strauss",
  ]);
});

test("Hervorhebung zeigt die Fundstelle im Originaltext, auch über Umlaute hinweg", () => {
  const [mueller] = filtere(eintraege("Jörg Müller"), "mueller");
  expect(mueller.bereich).toEqual([5, 11]);
  expect(trefferTeile("Jörg Müller", mueller.bereich)).toEqual([
    { text: "Jörg ", treffer: false },
    { text: "Müller", treffer: true },
  ]);
  const [strauss] = filtere(eintraege("Petra Strauß"), "auss");
  expect(trefferTeile("Petra Strauß", strauss.bereich)).toEqual([
    { text: "Petra Str", treffer: false },
    { text: "auß", treffer: true },
  ]);
  const [baer] = filtere(eintraege("Bär"), "bae");
  expect(trefferTeile("Bär", baer.bereich)).toEqual([
    { text: "Bä", treffer: true },
    { text: "r", treffer: false },
  ]);
  expect(trefferTeile("Bär", null)).toEqual([{ text: "Bär", treffer: false }]);
});

test("Buchstabensprung: erster Treffer, wiederholtes Tippen geht weiter, Umlaute als ganze Zeichen", () => {
  const liste = eintraege(
    "öffentlich",
    "intern",
    "vertraulich",
    "streng vertraulich",
    "Verschlusssache",
  );
  // Nichts aktiv: erster passender Eintrag.
  expect(sprungZiel(liste, "v", -1)).toBe(2);
  // Derselbe Buchstabe noch einmal: nächster Treffer, am Ende wieder vorn.
  expect(sprungZiel(liste, "v", 2)).toBe(4);
  expect(sprungZiel(liste, "v", 4)).toBe(2);
  // Schnell zweimal derselbe Buchstabe verhält sich wie langsames Wiederholen.
  expect(sprungZiel(liste, "vv", 2)).toBe(4);
  // Mehrere Buchstaben: Anfang des Eintrags, ab dem aktiven Eintrag.
  expect(sprungZiel(liste, "st", 0)).toBe(3);
  expect(sprungZiel(liste, "ver", 2)).toBe(2);
  expect(sprungZiel(liste, "vers", 2)).toBe(4);
  // „ö“ und „oe“ finden „öffentlich“, „o“ allein nicht (nur halber Umlaut).
  expect(sprungZiel(liste, "ö", 2)).toBe(0);
  expect(sprungZiel(liste, "oe", 2)).toBe(0);
  expect(sprungZiel(liste, "o", 2)).toBe(-1);
  expect(sprungZiel(eintraege("Öl", "Oase"), "o", -1)).toBe(1);
  // Ohne Treffer: -1; gesperrte Einträge werden übersprungen.
  expect(sprungZiel(liste, "x", 1)).toBe(-1);
  expect(
    sprungZiel(
      [{ label: "Vorlage A", disabled: true }, { label: "Vorlage B" }],
      "v",
      -1,
    ),
  ).toBe(1);
});
