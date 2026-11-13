// Tests zum Installer mit Modellen (lva#574).
//
// 1. Wächter in nsis/installer.nsi: ungesetzte oder leere LVA_VORINSTALLIERT → schlanker Bau;
//    gesetzt, aber ohne Nutzlast → makensis bricht ab; mit Nutzlast → Modellzweig aktiv.
//    Der Wächterblock wird aus der Vorlage geschnitten und allein übersetzt (die ganze Vorlage
//    braucht Tauris Handlebars-Werte und Plugins). Ohne makensis wird dieser Teil übersprungen.
// 2. Lizenzaussage in docs/INSTALLER-MODELLE.md: „kostenlos“ darf nicht als Freigabe für
//    CC BY-NC gelten (Gegenbeispiel: kostenlose, geschäftlich motivierte Weitergabe).
//
// Aufruf aus apps/local-voice: node --test scripts/installer-vorinstalliert.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";

const here = dirname(fileURLToPath(import.meta.url));
const nsi = readFileSync(join(here, "../src-tauri/nsis/installer.nsi"), "utf8");
const doc = readFileSync(join(here, "../../../docs/INSTALLER-MODELLE.md"), "utf8");
const portable = readFileSync(join(here, "../../../docs/PORTABLE.md"), "utf8");

const findMakensis = () => {
  const candidates = ["makensis"];
  if (process.env.LOCALAPPDATA) {
    candidates.push(join(process.env.LOCALAPPDATA, "tauri", "NSIS", "makensis.exe"));
    candidates.push(join(process.env.LOCALAPPDATA, "tauri", "NSIS", "Bin", "makensis.exe"));
  }
  for (const c of candidates) {
    const r = spawnSync(c, ["-VERSION"], { encoding: "utf8" });
    if (r.status === 0) return c;
  }
  return null;
};
const makensis = findMakensis();

const guardBlock = () => {
  const start = nsi.indexOf("; --- VORINSTALLIERT --- Installer mit Modellen");
  const end = nsi.indexOf("; --- END VORINSTALLIERT ---", start);
  assert.ok(start >= 0 && end > start, "Wächterblock in installer.nsi gefunden");
  return nsi.slice(start, end);
};

const compile = (envValue) => {
  const dir = mkdtempSync(join(tmpdir(), "lva-nsis-"));
  const script = join(dir, "t.nsi");
  writeFileSync(
    script,
    [
      "Unicode true",
      `OutFile "${join(dir, "t.exe")}"`,
      guardBlock(),
      "!ifdef VORINSTALLIERT",
      '  !warning "MODELLZWEIG_AKTIV"',
      "!endif",
      "Section -a",
      "SectionEnd",
      "",
    ].join("\n"),
    "utf8",
  );
  const env = { ...process.env };
  delete env.LVA_VORINSTALLIERT;
  if (envValue !== undefined) env.LVA_VORINSTALLIERT = envValue;
  const r = spawnSync(makensis, [script], { encoding: "utf8", env });
  return { status: r.status, out: `${r.stdout}\n${r.stderr}` };
};

const payload = () => {
  const dir = mkdtempSync(join(tmpdir(), "lva-nutzlast-"));
  writeFileSync(join(dir, "vorinstalliert.nsh"), '!define VORINST_TEST "1"\n', "utf8");
  return dir;
};

const skip = makensis ? false : "makensis nicht gefunden";

test("ungesetzte Variable baut schlank", { skip }, () => {
  const r = compile(undefined);
  assert.equal(r.status, 0, r.out);
  assert.ok(!r.out.includes("MODELLZWEIG_AKTIV"), r.out);
});

test("leere Variable baut schlank", { skip }, () => {
  const r = compile("");
  assert.equal(r.status, 0, r.out);
  assert.ok(!r.out.includes("MODELLZWEIG_AKTIV"), r.out);
});

test("gesetzte Variable ohne Ordner bricht ab", { skip }, () => {
  const missing = join(tmpdir(), `lva-gibt-es-nicht-${randomUUID()}`);
  assert.ok(!existsSync(missing));
  const r = compile(missing);
  assert.notEqual(r.status, 0, r.out);
  assert.match(r.out, /vorinstalliert\.nsh fehlt/);
});

test("gesetzte Variable ohne vorinstalliert.nsh bricht ab", { skip }, () => {
  const dir = mkdtempSync(join(tmpdir(), "lva-leer-"));
  mkdirSync(join(dir, "nemotron"));
  const r = compile(dir);
  assert.notEqual(r.status, 0, r.out);
  assert.match(r.out, /vorinstalliert\.nsh fehlt/);
});

test("gesetzte Variable mit Nutzlast aktiviert den Modellzweig", { skip }, () => {
  const r = compile(payload());
  assert.equal(r.status, 0, r.out);
  assert.ok(r.out.includes("MODELLZWEIG_AKTIV"), r.out);
});

test("Lizenzaussage: kostenlos ist keine Freigabe für CC BY-NC", () => {
  for (const text of [doc, portable]) {
    assert.doesNotMatch(text, /kostenlose[n]? Weitergabe im öffentlichen GitHub-Release ist dadurch gedeckt/);
    assert.doesNotMatch(text, /kostenlosen öffentlichen Release ist durch CC BY-NC 4\.0 gedeckt/);
  }
  assert.match(portable, /kostenlos ist, genügt dafür allein nicht/);
  assert.match(doc, /nur zu nicht-kommerziellen Zwecken/);
  assert.match(doc, /kostenlos ist, genügt dafür allein nicht/);
  assert.match(doc, /kostenlose, aber geschäftlich motivierte Weitergabe[^.]*ist nicht gedeckt/);
});

test("Lizenzaussage: Abwählen beim Empfänger heilt keine Weitergabe", () => {
  // Gegenbeispiel: vollständiger Installer als Beigabe weitergegeben, Empfänger wählt Deutsch ab.
  const weitergabe = doc.match(/\*\*Weitergabe\.\*\*([^\n]*)/);
  assert.ok(weitergabe, "Absatz zur Weitergabe vorhanden");
  assert.match(weitergabe[1], /Daran ändert nichts, wenn der Empfänger die Komponente[^.]*abwählt/);
  assert.match(weitergabe[1], /-OhnePhononDe/);
  assert.match(weitergabe[1], /gesonderte Erlaubnis/);
  // Abwählen/Entfernen darf nur als Weg für die Nutzung stehen, nie als Lösung der Weitergabe.
  assert.doesNotMatch(doc, /Für solche Fälle das deutsche Modell nach der Installation entfernen/);
  const lic = readFileSync(join(here, "../src-tauri/nsis/LIZENZEN-MODELLE.txt"), "utf8");
  assert.match(lic, /auch wenn der Empfänger es abwählt/);
});
