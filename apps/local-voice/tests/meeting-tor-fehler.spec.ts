import { test, expect } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";
import {
  metadataErrorKey,
  translateMeetingError,
} from "../src/components/settings/meetings/meetingErrors";

// lva#552: Das Einwilligungs-Tor der Besprechungsplanung antwortet `nur_notizen: <grund>`
// (Ereignisse tragen `nur_notizen` ohne Grund), bei einem Fehler der Datenbank
// `nur_notizen: tor_nicht_pruefbar`. Die Oberflaeche zeigte den Rohtext; jetzt steht dort ein
// Satz in der Sprache der App. Reine Textabbildung: kein Browser, keine Tauri-Attrappe.

const locale = (lang: string) =>
  JSON.parse(
    fs.readFileSync(
      path.resolve(process.cwd(), `src/i18n/locales/${lang}/translation.json`),
      "utf-8",
    ),
  );

/** `t` auf einer Locale-Datei: `FEHLT:<schluessel>`, wenn es den Schluessel nicht gibt. */
const tOf = (lang: string) => {
  const root = locale(lang);
  return ((key: string) =>
    key.split(".").reduce<any>((node, part) => node?.[part], root) ??
    `FEHLT:${key}`) as any;
};

const GRUENDE = [
  "nicht_geplant",
  "abgelehnt",
  "offen",
  "tor_nicht_pruefbar",
] as const;

test("nur_notizen und tor_nicht_pruefbar werden in Deutsch und Englisch uebersetzt", () => {
  for (const lang of ["de", "en"]) {
    const text = (code: string) => translateMeetingError(code, tOf(lang));
    const codes = [
      "nur_notizen",
      ...GRUENDE.map((g) => `nur_notizen: ${g}`),
      "nur_notizen: ohne_modellarbeit",
      "tor_nicht_pruefbar",
    ];
    for (const code of codes) {
      const out = text(code);
      expect(out, `${lang}: ${code}`).not.toContain("FEHLT:");
      expect(out, `${lang}: ${code}`).not.toContain("nur_notizen");
      expect(out, `${lang}: ${code}`).not.toContain("tor_nicht_pruefbar");
      expect(out.length, `${lang}: ${code}`).toBeGreaterThan(20);
    }
    // Jeder Grund hat seinen eigenen Satz ...
    expect(new Set(GRUENDE.map((g) => text(`nur_notizen: ${g}`))).size).toBe(
      GRUENDE.length,
    );
    // ... ein unbekannter oder fehlender Grund den allgemeinen ...
    expect(text("nur_notizen: ohne_modellarbeit")).toBe(text("nur_notizen"));
    expect(text("nur_notizen:")).toBe(text("nur_notizen"));
    // ... und `tor_nicht_pruefbar` ist mit und ohne Vorsatz derselbe Satz, auch mit Detail dahinter.
    const nichtPruefbar = text("nur_notizen: tor_nicht_pruefbar");
    expect(text("tor_nicht_pruefbar")).toBe(nichtPruefbar);
    expect(
      text("nur_notizen: tor_nicht_pruefbar: sql: database is locked"),
    ).toBe(nichtPruefbar);
    expect(nichtPruefbar).not.toContain("database is locked");
  }
});

test("die deutschen Tor-Texte nutzen echte Umlaute", () => {
  const t = tOf("de");
  const alle = ["nur_notizen", ...GRUENDE.map((g) => `nur_notizen: ${g}`)].map(
    (code) => translateMeetingError(code, t),
  );
  const text = alle.join(" ");
  expect(text).toContain("Für");
  expect(text).toContain("prüfen");
  expect(text).toContain("ließen");
  for (const falsch of ["Fuer", "pruefen", "liessen", "ueber"]) {
    expect(text, falsch).not.toContain(falsch);
  }
});

// Codex-Review zu lva#706: Die Nachschlagetabellen erbten den Objektprototyp, `nur_notizen: constructor`
// lieferte deshalb eine Funktion statt eines Satzes. Nur eigene Eintraege zaehlen.
const PROTOTYP_NAMEN = [
  "constructor",
  "toString",
  "hasOwnProperty",
  "valueOf",
  "__proto__",
  "isPrototypeOf",
];

test("Namen aus dem Objektprototyp sind keine Fehlercodes", () => {
  for (const lang of ["de", "en"]) {
    const t = tOf(lang);
    const allgemein = translateMeetingError("nur_notizen", t);
    for (const name of PROTOTYP_NAMEN) {
      // Grund hinter `nur_notizen:`: der allgemeine Satz, nie eine Funktion oder `[object ...]`.
      const grund = translateMeetingError(`nur_notizen: ${name}`, t);
      expect(typeof grund, `${lang}: nur_notizen: ${name}`).toBe("string");
      expect(grund, `${lang}: nur_notizen: ${name}`).toBe(allgemein);
      // Als Code selbst: unbekannt, also der Rohtext (wie jeder andere unbekannte Code).
      const roh = translateMeetingError(name, t);
      expect(roh, `${lang}: ${name}`).toBe(name);
      const mitDetail = translateMeetingError(`${name}: Detail`, t);
      expect(mitDetail, `${lang}: ${name}: Detail`).toBe(`${name}: Detail`);
      // Metadaten-Fehler: der allgemeine Schluessel.
      expect(metadataErrorKey(name), `metadata ${name}`).toBe(
        "meetings.metadata.errors.unknown",
      );
    }
  }
});
