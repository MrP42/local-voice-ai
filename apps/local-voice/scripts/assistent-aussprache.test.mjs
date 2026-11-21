// G2 Teil 2 (lva#556): Ausspracheregeln im Editor der Assistenten. Reine Logik (Liste ändern, Befunde je Zeile, Feld nie als `[]`,
// unbekannte Felder bleiben) und der Weg durch Kopieren, Umbenennen, Export und Import. Der Kern (`wendeAusspracheAn`, die
// Prüfung) wird nur benutzt. Die Sprachausgabe im Gespräch ist hier bewusst nicht berührt: sie ist nicht Teil dieses Pakets.
// Lauf: node --test scripts/assistent-aussprache.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import * as modul from "node:module";
import test from "node:test";

modul.register("./test-stubs/hooks.mjs", import.meta.url);
globalThis.window = { addEventListener() {}, removeEventListener() {} };

const BASIS = "../src/components/voice-call/";
const kern = await import(`${BASIS}assistenten/index.ts`);
const editor = await import(`${BASIS}profil/editor.ts`);
const verwalten = await import(`${BASIS}profil/verwalten.ts`);
const { pfadText, fehlerZeile } =
  await import("../src/components/assistenten/fehler.ts");

const {
  AUSSPRACHE_MAX,
  AUSSPRACHE_TEXT_MAX,
  kodiereProfil,
  neuesProfil,
  parseProfil,
  pruefeProfil,
} = kern;

const leer = () => neuesProfil("eigen-a", "Eigen A");
const regel = (wort, aussprache, extra) =>
  extra ? { wort, aussprache, extra } : { wort, aussprache };
const mitRegeln = (...liste) => editor.mitAussprache(leer(), liste);
const text = (p) => kodiereProfil(p);
const hatFeld = (p) => Object.hasOwn(p, "aussprache");
const nachweis = (p) => JSON.parse(text(p));
const nenne = (fehler) => fehler.map((f) => `${f.pfad}:${f.code}`);

/** Eine Attrappe von `t`: gibt Schlüssel und Werte zurück, damit der Aufbau der Texte prüfbar ist. */
const t = (k, o) => (o ? `${k}${JSON.stringify(o)}` : k);

// ===================================================================== Feld fehlt, Feld nie als []

test("Ein Profil ohne Regeln hat kein Feld und schreibt dieselben Bytes wie vor dem Feld", () => {
  const p = leer();
  assert.equal(hatFeld(p), false);
  assert.deepEqual(editor.ausspracheListe(p), []);
  assert.equal(Object.hasOwn(nachweis(p), "aussprache"), false);
  assert.equal(editor.zuSchreiben(p), p);
  // Andere Änderungen am Entwurf legen das Feld nie an.
  for (const q of [
    editor.mit(p, { name: "X" }),
    editor.mitVerhalten(p, { stil: "woertlich" }),
    editor.mitWissen(p, { alleBereiche: true }),
    editor.bereichSchalten(p, "meetings", true),
  ])
    assert.equal(hatFeld(q), false);
});

test("Hinzufügen und wieder Entfernen der letzten Regel: das Feld fällt weg, die Bytes sind wie vorher, nie []", () => {
  const p = leer();
  const mit = editor.ausspracheHinzu(p);
  assert.equal(hatFeld(mit), true);
  assert.deepEqual(mit.aussprache, [{ wort: "", aussprache: "" }]);
  const zurueck = editor.ausspracheEntfernen(mit, 0);
  assert.equal(hatFeld(zurueck), false);
  assert.equal(text(zurueck), text(p));
  assert.equal(text(zurueck).includes("aussprache"), false);
  assert.equal(editor.istGeaendert(p, zurueck), false);
  // Der Ursprung wurde nie verändert.
  assert.equal(hatFeld(p), false);
  assert.deepEqual(mit.aussprache.length, 1);
});

test("mitAussprache([]) und zuSchreiben schreiben nie [] (auch nicht bei einer leeren Liste aus der Datei)", () => {
  assert.equal(
    hatFeld(editor.mitAussprache(mitRegeln(regel("a", "b")), [])),
    false,
  );
  const roh = text(leer()).replace(/\n}\n$/, ',\n  "aussprache": []\n}\n');
  const gelesen = parseProfil(roh);
  assert.equal(gelesen.ok, true, JSON.stringify(gelesen.fehler));
  // Der Kern behält eine vorhandene leere Liste, wie sie ist ...
  assert.deepEqual(gelesen.profil.aussprache, []);
  assert.ok(text(gelesen.profil).includes('"aussprache": []'));
  // ... der Editor schreibt sie nie: ohne Feld, und das gilt nicht als Änderung.
  const schreib = editor.zuSchreiben(gelesen.profil);
  assert.equal(hatFeld(schreib), false);
  assert.equal(text(schreib).includes("aussprache"), false);
  assert.equal(editor.istGeaendert(gelesen.profil, gelesen.profil), false);
  assert.equal(editor.istGeaendert(gelesen.profil, schreib), false);
  // Mit einer anderen Änderung wird ohne das Feld geschrieben.
  const anders = editor.mit(gelesen.profil, { name: "Neu" });
  assert.equal(editor.istGeaendert(gelesen.profil, anders), true);
  assert.equal(text(editor.zuSchreiben(anders)).includes("aussprache"), false);
  // Abgleich: die Datei ohne das Feld nach dem eigenen Speichern gilt nicht als Konflikt.
  const basis = { profil: gelesen.profil, text: roh };
  const jetzt = { profil: schreib, text: text(schreib) };
  assert.equal(
    editor.gleicheAb(basis, jetzt, gelesen.profil).art,
    "uebernommen",
  );
});

test("Kein Ablauf von Änderungen lässt [] in die Ausgabe kommen", () => {
  let p = leer();
  const schritte = [
    (x) => editor.ausspracheHinzu(x),
    (x) => editor.ausspracheAendern(x, 0, "wort", "SQL"),
    (x) => editor.ausspracheAendern(x, 0, "aussprache", "Siekwel"),
    (x) => editor.ausspracheHinzu(x),
    (x) => editor.ausspracheVerschieben(x, 1, -1),
    (x) => editor.ausspracheEntfernen(x, 0),
    (x) => editor.ausspracheEntfernen(x, 0),
    (x) => editor.ausspracheEntfernen(x, 0),
    (x) => editor.ausspracheHinzu(x),
    (x) => editor.ausspracheEntfernen(x, 0),
  ];
  for (const s of schritte) {
    p = s(p);
    assert.equal(
      text(editor.zuSchreiben(p)).includes('"aussprache": []'),
      false,
    );
    if (hatFeld(p)) assert.ok(p.aussprache.length > 0);
  }
  assert.equal(hatFeld(p), false);
});

// ===================================================================== Liste ändern

test("Hinzufügen hängt eine leere Regel an, bis AUSSPRACHE_MAX; darüber ändert sich nichts", () => {
  let p = leer();
  for (let i = 0; i < AUSSPRACHE_MAX; i += 1) p = editor.ausspracheHinzu(p);
  assert.equal(p.aussprache.length, AUSSPRACHE_MAX);
  assert.equal(AUSSPRACHE_MAX, 50);
  assert.equal(editor.ausspracheHinzu(p), p);
  assert.equal(editor.GRENZEN.aussprache, 50);
  assert.equal(editor.GRENZEN.ausspracheText, AUSSPRACHE_TEXT_MAX);
});

test("Ändern, Entfernen und Verschieben ändern nur die Zeile und lassen den Ursprung unberührt", () => {
  const a = regel("SQL", "Siekwel");
  const b = regel("Wolff.ai", "Wolf Punkt A I");
  const c = regel("DSGVO", "Daten Schutz");
  const p = mitRegeln(a, b, c);
  const q = editor.ausspracheAendern(p, 1, "aussprache", "Wolf Punkt Ai");
  assert.deepEqual(q.aussprache, [a, regel("Wolff.ai", "Wolf Punkt Ai"), c]);
  assert.deepEqual(p.aussprache, [a, b, c]);
  assert.deepEqual(editor.ausspracheEntfernen(p, 1).aussprache, [a, c]);
  assert.deepEqual(editor.ausspracheVerschieben(p, 2, -1).aussprache, [
    a,
    c,
    b,
  ]);
  assert.deepEqual(editor.ausspracheVerschieben(p, 0, 1).aussprache, [b, a, c]);
  // Am Rand und außerhalb ändert sich nichts.
  assert.equal(editor.ausspracheVerschieben(p, 0, -1), p);
  assert.equal(editor.ausspracheVerschieben(p, 2, 1), p);
  assert.equal(editor.ausspracheVerschieben(p, 9, 1), p);
  assert.equal(editor.ausspracheEntfernen(p, -1), p);
  assert.equal(editor.ausspracheEntfernen(p, 3), p);
  assert.equal(editor.ausspracheAendern(p, 3, "wort", "x"), p);
  // Die Reihenfolge ist die der Datei: es wird nie sortiert.
  assert.deepEqual(
    nachweis(p).aussprache.map((e) => e.wort),
    ["SQL", "Wolff.ai", "DSGVO"],
  );
  assert.deepEqual(
    nachweis(editor.ausspracheVerschieben(p, 2, -1)).aussprache.map(
      (e) => e.wort,
    ),
    ["SQL", "DSGVO", "Wolff.ai"],
  );
});

test("Unbekannte Felder einer Regel und des Profils bleiben beim Ändern, Verschieben und Entfernen anderer Regeln", () => {
  const roh = text(leer()).replace(
    /\n}\n$/,
    `,\n  "aussprache": [\n    {"aussprache": "Siekwel", "notiz": {"von": "Anna", "n": 2.50}, "wort": "SQL"},\n    {"aussprache": "Be-urlaubung", "wort": "BU"}\n  ],\n  "zukunft": [1, 2.50]\n}\n`,
  );
  const gelesen = parseProfil(roh);
  assert.equal(gelesen.ok, true, JSON.stringify(gelesen.fehler));
  const p = gelesen.profil;
  assert.equal(p.aussprache[0].extra.length, 1);
  const verschoben = editor.ausspracheVerschieben(p, 0, 1);
  const geaendert = editor.ausspracheAendern(
    verschoben,
    1,
    "aussprache",
    "Sieh Kwell",
  );
  const aus = text(geaendert);
  assert.ok(aus.includes('"notiz"'));
  assert.ok(aus.includes("2.50"));
  assert.ok(aus.includes('"zukunft"'));
  const tief = nachweis(geaendert).aussprache;
  assert.deepEqual(
    tief.map((e) => e.wort),
    ["BU", "SQL"],
  );
  assert.deepEqual(tief[1].notiz, { von: "Anna", n: 2.5 });
  assert.equal(tief[1].aussprache, "Sieh Kwell");
  // Ändern allein verliert nichts, und Entfernen einer anderen Regel auch nicht.
  const ohneBU = editor.ausspracheEntfernen(p, 1);
  assert.ok(text(ohneBU).includes('"notiz"'));
  // Der Entwurf, den der Editor schreibt, besteht die Prüfung des Kerns und liest sich wieder gleich.
  const wieder = parseProfil(text(geaendert));
  assert.equal(wieder.ok, true);
  assert.equal(text(wieder.profil), aus);
});

// ===================================================================== Befunde je Zeile

test("Eine eben angelegte Regel ist (noch) nicht speicherbar: Länge je Feld, mit Pfad und Zeile", () => {
  const p = editor.ausspracheHinzu(leer());
  const fehler = editor.pruefe(p);
  assert.deepEqual(nenne(fehler).sort(), [
    "aussprache[0].aussprache:laenge",
    "aussprache[0].wort:laenge",
  ]);
  const b = editor.ausspracheBefunde(fehler);
  assert.deepEqual(b.liste, []);
  assert.deepEqual(b.zeilen[0], {
    wort: ["laenge"],
    aussprache: ["laenge"],
    zeile: [],
  });
  assert.deepEqual(Object.keys(editor.fehlerJeFeld(fehler)), ["aussprache"]);
});

test("Länge, Zeichen und Dublette melden die Fehler des Kerns an der richtigen Zeile und dem richtigen Feld", () => {
  const p = mitRegeln(
    regel("SQL", "Siekwel"),
    regel("x".repeat(AUSSPRACHE_TEXT_MAX + 1), "ok"),
    regel("[lachen]", "ok"),
    regel("SQL", "nochmal"),
    regel("ok", "a<b"),
    regel(
      "\u{1F600}".repeat(AUSSPRACHE_TEXT_MAX),
      "60 Zeichen außerhalb der BMP",
    ),
    regel("zeile\nzwei", "ok"),
  );
  const fehler = editor.pruefe(p);
  assert.deepEqual(nenne(fehler), [
    "aussprache[1].wort:laenge",
    "aussprache[2].wort:muster",
    "aussprache[3].wort:doppelt",
    "aussprache[4].aussprache:muster",
    "aussprache[6].wort:muster",
  ]);
  const b = editor.ausspracheBefunde(fehler);
  assert.deepEqual(Object.keys(b.zeilen).map(Number), [1, 2, 3, 4, 6]);
  assert.deepEqual(b.zeilen[3].wort, ["doppelt"]);
  assert.deepEqual(b.zeilen[4].aussprache, ["muster"]);
  assert.deepEqual(b.zeilen[0], undefined);
  // 60 Zeichen außerhalb der BMP sind erlaubt (Unicode-Skalare zählen, nicht UTF-16-Einheiten).
  assert.equal(b.zeilen[5], undefined);
});

test("Nach dem Verschieben folgen die Befunde den Zeilen: die Dublette liegt immer an der späteren", () => {
  const p = mitRegeln(regel("SQL", "a"), regel("x", "b"), regel("SQL", "c"));
  assert.deepEqual(nenne(editor.pruefe(p)), ["aussprache[2].wort:doppelt"]);
  const eins = editor.ausspracheVerschieben(p, 2, -1);
  assert.deepEqual(
    eins.aussprache.map((e) => e.aussprache),
    ["a", "c", "b"],
  );
  assert.deepEqual(nenne(editor.pruefe(eins)), ["aussprache[1].wort:doppelt"]);
  // Steht die zweite SQL-Regel jetzt vor der ersten, ist die frühere Regel die doppelte.
  const zwei = editor.ausspracheVerschieben(eins, 1, -1);
  assert.deepEqual(
    zwei.aussprache.map((e) => e.aussprache),
    ["c", "a", "b"],
  );
  assert.deepEqual(nenne(editor.pruefe(zwei)), ["aussprache[1].wort:doppelt"]);
  const b = editor.ausspracheBefunde(editor.pruefe(zwei));
  assert.deepEqual(b.zeilen[1].wort, ["doppelt"]);
  // Die Befunde stehen an den Zeilen, nicht an den Regeln: nach dem Entfernen der Dublette ist alles gut.
  assert.deepEqual(editor.pruefe(editor.ausspracheEntfernen(zwei, 1)), []);
});

test("Mehr als 50 Regeln: zu_viele an der Liste, die Zeilen werden trotzdem geprüft", () => {
  const liste = Array.from({ length: 51 }, (_, i) => regel(`w${i}`, `a${i}`));
  liste[3] = regel("", "x");
  const p = editor.mitAussprache(leer(), liste);
  const b = editor.ausspracheBefunde(editor.pruefe(p));
  assert.deepEqual(b.liste, ["zu_viele"]);
  assert.deepEqual(b.zeilen[3].wort, ["laenge"]);
});

test("Befunde, die der Editor nicht kennt, gehören der Zeile; Fremdes in anderen Feldern wird nicht dazugezählt", () => {
  const b = editor.ausspracheBefunde([
    { pfad: "aussprache[1]", code: "typ" },
    { pfad: "aussprache[1].notiz", code: "json" },
    { pfad: "aussprache", code: "typ" },
    { pfad: "name", code: "laenge" },
    { pfad: "ausspracheXY", code: "typ" },
  ]);
  assert.deepEqual(b.liste, ["typ"]);
  assert.deepEqual(b.zeilen[1].zeile, ["typ", "json"]);
  assert.equal(Object.keys(b.zeilen).length, 1);
  assert.equal(editor.feldVon("aussprache[2].wort"), "aussprache");
});

test("Die Anzeige der Fehlerpfade nennt Regel und Feld", () => {
  assert.equal(pfadText(t, "aussprache"), "assistenten.pfad.aussprache");
  assert.equal(
    pfadText(t, "aussprache[1]"),
    'assistenten.pfad.aussprache, assistenten.pfad.aussprache_regel{"n":2}',
  );
  assert.equal(
    pfadText(t, "aussprache[0].wort"),
    'assistenten.pfad.aussprache, assistenten.pfad.aussprache_regel{"n":1}, assistenten.pfad.aussprache_wort',
  );
  assert.equal(
    pfadText(t, "aussprache[4].aussprache"),
    'assistenten.pfad.aussprache, assistenten.pfad.aussprache_regel{"n":5}, assistenten.pfad.aussprache_aussprache',
  );
  assert.match(
    fehlerZeile(t, { pfad: "aussprache[0].wort", code: "muster" }),
    /aussprache_wort: assistenten.code.muster$/,
  );
  // Ein unbekannter Pfad bleibt, wie er ist.
  assert.equal(pfadText(t, "aussprache[0].notiz"), "aussprache[0].notiz");
});

// ===================================================================== Verwalten: Kopie, Export, Import, Umbenennen

test("Kopieren, Umbenennen, Export und Import tragen die Regeln unverändert mit (ein Profil ohne Regeln bekommt keine)", () => {
  const a = {
    vorlagen: kern.VORLAGEN,
    eigene: [],
    abgewiesen: [],
    aktivId: "alles",
    aktivBelegt: false,
    ladefehler: null,
  };
  const p = mitRegeln(
    regel("SQL", "Siekwel"),
    regel("Wolff.ai", "Wolf Punkt A I"),
  );
  const kopie = verwalten.kopieren(p, "Kopie", a);
  assert.equal(kopie.ok, true, JSON.stringify(kopie.fehler));
  assert.deepEqual(nachweis(kopie.profil).aussprache, nachweis(p).aussprache);
  assert.notEqual(kopie.profil.aussprache, p.aussprache);
  assert.notEqual(kopie.profil.aussprache[0], p.aussprache[0]);
  const umbenannt = verwalten.umbenennen(p, "Neuer Name");
  assert.equal(umbenannt.ok, true);
  assert.deepEqual(
    nachweis(umbenannt.profil).aussprache,
    nachweis(p).aussprache,
  );
  const datei = verwalten.exportieren(p);
  assert.equal(datei.text, text(p));
  assert.equal(nachweis(p).aussprache.length, 2);
  // Import: dieselben Regeln, bei belegter Id mit neuer Id, und ohne Feld bleibt es ohne.
  const ein = verwalten.importieren(datei.text, a);
  assert.equal(ein.ok, true);
  assert.equal(text(ein.profil), datei.text);
  const belegt = verwalten.importieren(datei.text, {
    ...a,
    eigene: [{ profil: p, text: datei.text, datei: "eigen-a.json" }],
  });
  assert.equal(belegt.ok, true);
  assert.equal(belegt.idGeaendert, true);
  assert.deepEqual(nachweis(belegt.profil).aussprache, nachweis(p).aussprache);
  const ohne = verwalten.importieren(text(leer()), a);
  assert.equal(ohne.ok, true);
  assert.equal(hatFeld(ohne.profil), false);
  assert.equal(hatFeld(verwalten.kopieren(leer(), "K", a).profil), false);
});

test("Eine Datei mit ungültiger Aussprache wird beim Import abgewiesen, mit Pfad, und nichts wird repariert", () => {
  const a = {
    vorlagen: kern.VORLAGEN,
    eigene: [],
    abgewiesen: [],
    aktivId: "alles",
    aktivBelegt: false,
    ladefehler: null,
  };
  const mit = (liste) =>
    text(leer()).replace(
      /\n}\n$/,
      `,\n  "aussprache": ${JSON.stringify(liste)}\n}\n`,
    );
  const f = (liste) => verwalten.importieren(mit(liste), a);
  assert.deepEqual(nenne(f([{ wort: "[x]", aussprache: "y" }]).fehler), [
    "aussprache[0].wort:muster",
  ]);
  assert.deepEqual(nenne(f([{ wort: "a" }]).fehler), [
    "aussprache[0].aussprache:fehlt",
  ]);
  assert.deepEqual(nenne(f([{ wort: "a", aussprache: "b" }, 5]).fehler), [
    "aussprache[1]:typ",
  ]);
  assert.deepEqual(nenne(f("nein").fehler), ["aussprache:typ"]);
  assert.deepEqual(
    nenne(
      f([
        { wort: "a", aussprache: "b" },
        { wort: "a", aussprache: "c" },
      ]).fehler,
    ),
    ["aussprache[1].wort:doppelt"],
  );
  assert.equal(f([{ wort: "a", aussprache: "b" }]).ok, true);
  assert.equal(f([]).ok, true);
});

test("Der Kern prüft denselben Entwurf wie der Editor (kein eigener Maßstab im Editor)", () => {
  const p = mitRegeln(regel("a", "b"), regel("", "c"));
  assert.deepEqual(editor.pruefe(p), pruefeProfil(p));
});
