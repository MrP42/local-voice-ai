// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/projekte.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  alsKlasse,
  auspacken,
  datumKurz,
  eintragArt,
  freigabeAnfragen,
  freigabeKontext,
  hostVon,
  projektEintrag,
  projektKurz,
  rechercheErgebnis,
  verweisZiel,
} from "../../src/lib/projekte.ts";

// Nur synthetische Daten (example.*).
const SUITE = {
  id: "prj_01HZZZZZZZZZZZZZZZZZZZZZZZ",
  title: "Wärmewende",
  goal: null,
  dataClass: "PERSONAL",
  specialCategory: false,
  archived: false,
  webResearchEnabled: true,
  entryCount: 2,
  revision: "1",
};

test("Datenklasse: unbekannte Werte werden streng als PERSONAL gelesen", () => {
  assert.equal(alsKlasse("INTERNAL"), "INTERNAL");
  assert.equal(alsKlasse("quatsch"), "PERSONAL");
  assert.equal(alsKlasse(null), "PERSONAL");
});

test("auspacken liefert Daten oder wirft den Fehlertext", () => {
  assert.equal(auspacken({ status: "ok", data: 5 }), 5);
  assert.throws(() => auspacken({ status: "error", error: "nein" }), /nein/);
});

test("Projekt-Kurzsicht übernimmt Klasse, Web-Freigabe und Zähler", () => {
  const k = projektKurz(SUITE);
  assert.equal(k.dataClass, "PERSONAL");
  assert.equal(k.webResearchEnabled, true);
  assert.equal(k.entryCount, 2);
});

test("Recherchen liegen als document mit Recherche-Verweis und werden research", () => {
  assert.equal(
    eintragArt({ kind: "document", link: "wai://lva/recherche/rch_1" }),
    "research",
  );
  assert.equal(eintragArt({ kind: "document", link: null }), "document");
  assert.equal(eintragArt({ kind: "meeting", link: null }), "meeting");
  assert.equal(eintragArt({ kind: "book", link: null }), "book");
  assert.equal(eintragArt({ kind: "was-neues", link: null }), "document");
});

test("Verweise werden nur für die eigenen Arten und Zeichen gelesen", () => {
  assert.deepEqual(verweisZiel("wai://lva/besprechung/M1"), {
    art: "besprechung",
    id: "M1",
  });
  assert.deepEqual(verweisZiel("wai://lva/buch/book_17"), {
    art: "buch",
    id: "book_17",
  });
  assert.equal(verweisZiel("wai://corporate-llm/unterhaltung/c1"), null);
  assert.equal(verweisZiel("wai://lva/buch/../x"), null);
  assert.equal(verweisZiel(null), null);
});

test("Datum wird zu TT.MM. gekürzt", () => {
  assert.equal(datumKurz("2026-10-30T20:41:00Z"), "30.10.");
  assert.equal(datumKurz("kaputt"), undefined);
  assert.equal(datumKurz(null), undefined);
});

test("Eintrag ohne Leserecht bleibt ohne Titel und ist nicht verfügbar", () => {
  const e = projektEintrag({
    id: "e1",
    kind: "meeting",
    title: null,
    subtitle: null,
    originApp: null,
    dataClass: null,
    available: false,
    assignedAt: null,
    assignedBy: null,
    link: null,
  });
  assert.equal(e.available, false);
  assert.equal(e.title, undefined);
  assert.equal(e.originApp, "lva");
});

test("Host aus der Adresse, unlesbare Adressen bleiben stehen", () => {
  assert.equal(hostVon("https://www.example.org/a?b=1"), "www.example.org");
  assert.equal(hostVon("kein url"), "kein url");
});

test("Recherche-Ergebnis: Quellenart, Hinweis auf Budget, Host", () => {
  const e = rechercheErgebnis({
    id: "rch_1",
    question: "Wie lange gilt die Frist?",
    createdAt: "2026-10-30T20:41:00Z",
    dataClass: "PERSONAL",
    dataClassLabel: "personenbezogen",
    projectId: SUITE.id,
    webApproved: true,
    stoppedReason: "budget",
    model: "modell-x",
    answer: [{ text: "Bis 2028.", citations: [1], unsupported: false }],
    sources: [
      {
        index: 1,
        title: "Frist",
        url: "https://www.example.org/a",
        retrievedAt: "2026-10-30T20:41:00Z",
        excerpt: "bis 2028",
        sourceType: "amtlich",
      },
    ],
    contradictions: [],
    followUps: ["Wer ist betroffen?"],
    queries: ["frist"],
    readAloud: "Bis 2028.",
  });
  assert.equal(e.sources[0].sourceType, "official");
  assert.equal(e.sources[0].host, "www.example.org");
  assert.match(e.provenance, /1 Quelle/);
  assert.match(e.provenance, /Budget erreicht/);
});

test("Der Freigabedialog nennt die Frage und die Datenklasse; nichts ist vorgewählt", () => {
  const info = {
    id: "rch_1",
    question: "Wie lange gilt die Frist?",
    dataClass: "PERSONAL",
    dataClassLabel: "personenbezogen",
    projectId: SUITE.id,
    queries: [
      {
        id: "a1",
        text: "frist waermeplanung",
        hint: "Kontext personenbezogen",
      },
      { id: "a2", text: "frist gemeinden", hint: null },
    ],
  };
  assert.match(freigabeKontext(info), /Wie lange gilt die Frist\?/);
  assert.match(freigabeKontext(info), /personenbezogen/);
  const q = freigabeAnfragen(info);
  assert.deepEqual(
    q.map((x) => x.selected),
    [false, false],
  );
  assert.equal(q[1].hint, undefined);
});
