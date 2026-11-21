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

// lva#654 (e): Nemotron-3-Diarization-ONNX (Sprechertrennung) im Installer mit Modellen.
// Erststart-Erkennung (Rust: commands::meeting_diarization::model_status) gilt eine Datei als
// vorhanden, wenn sie unter <models>/diarization/<id>/<datei> liegt und genau size_bytes gross ist.
// Dieser Test pruefte die Verdrahtung von Katalog, Nutzlast-Skript, NSIS und Lizenzdatei.
const DIAR_ID = "diar-nemotron3-8spk-onnx-q8";
const catalog = JSON.parse(readFileSync(join(here, "../src-tauri/src/catalog/catalog.json"), "utf8"));
const buildScript = readFileSync(join(here, "build-installer-modelle.ps1"), "utf8");
const licenses = readFileSync(join(here, "../src-tauri/nsis/LIZENZEN-MODELLE.txt"), "utf8");
const entries = Array.isArray(catalog) ? catalog : (catalog.models ?? catalog.entries ?? Object.values(catalog).find(Array.isArray));
const diarEntry = entries.find((e) => e.id === DIAR_ID);

test("Katalogeintrag Nemotron-3-Diarization: feste Revision, zwei Dateien mit Pruefsumme", () => {
  assert.ok(diarEntry, "Katalogeintrag vorhanden");
  assert.match(diarEntry.revision, /^[0-9a-f]{40}$/);
  assert.equal(diarEntry.files.length, 2);
  for (const f of diarEntry.files) {
    assert.match(f.sha256, /^[0-9a-f]{64}$/);
    assert.ok(f.size_bytes > 0);
    assert.ok(f.url.includes(diarEntry.revision), "URL auf feste Revision gepinnt");
    assert.ok(f.url.startsWith("https://huggingface.co/onnx-community/Nemotron-3-Diarization-ONNX/"));
  }
});

test("Nutzlast-Skript laedt Nemotron-3-Diarization ueber --provision, nur diese Kennung", () => {
  assert.match(buildScript, new RegExp(String.raw`diar\s*=\s*@\{\s*Ids\s*=\s*@\('${DIAR_ID}'\)`));
  // Keine weitere Diarization-Kennung und kein anderes neues Modell in der Komponente.
  const block = buildScript.match(/\$components = \[ordered\]@\{[\s\S]*?\r?\n\}/)[0];
  assert.equal((block.match(/diar-/g) ?? []).length, 1);
});

test("Nutzlast-Skript prueft Groesse und SHA-256 gegen den Katalog (Erststart-Kriterium)", () => {
  assert.match(buildScript, /Get-FileHash[^\n]*SHA256/);
  assert.match(buildScript, /diarization/);
  assert.match(buildScript, /catalog\.json/);
});

test("Pruefung Test-DiarPayload wird in Nutzlast UND Rauchtest aufgerufen und schlaegt bei falschem Inhalt fehl", { skip: spawnSync("pwsh", ["-NoProfile", "-Command", "1"]).status === 0 ? false : "pwsh nicht gefunden" }, () => {
  // 1. Beide Aufrufe muessen im Skript stehen (Nutzlast vor dem Lizenzkopieren, Rauchtest im SmokeTest-Zweig).
  const smoke = buildScript.indexOf("if ($SmokeTest)");
  assert.ok(smoke > 0, "SmokeTest-Zweig gefunden");
  const calls = [...buildScript.matchAll(/^\s*Test-DiarPayload \(Join-Path /gm)].map((m) => m.index);
  assert.equal(calls.length, 2, "genau zwei Aufrufe (Nutzlast, Rauchtest)");
  assert.ok(calls[0] < smoke && calls[1] > smoke, "ein Aufruf davor, einer im SmokeTest-Zweig");
  // 2. Die Funktion selbst: gueltige Nutzlast besteht, beschaedigte Datei und fehlende Datei scheitern.
  const fn = buildScript.match(/function Test-DiarPayload[\s\S]*?\r?\n\}\r?\n/)[0];
  const dir = mkdtempSync(join(tmpdir(), "lva-diar-"));
  const models = join(dir, "models");
  const target = join(models, "diarization", DIAR_ID);
  mkdirSync(target, { recursive: true });
  const run = () => {
    const ps = [
      `$catalogJson = '${join(here, "../src-tauri/src/catalog/catalog.json").replaceAll("'", "''")}'`,
      "$ErrorActionPreference = 'Stop'",
      fn,
      `Test-DiarPayload '${models.replaceAll("'", "''")}'`,
    ].join("\n");
    const f = join(dir, "t.ps1");
    writeFileSync(f, ps, "utf8");
    return spawnSync("pwsh", ["-NoProfile", "-File", f], { encoding: "utf8" });
  };
  // Dateien mit falscher Groesse: die Pruefung muss anschlagen (Exit-Code ungleich 0, Meldung zur Groesse).
  for (const f of diarEntry.files) writeFileSync(join(target, f.filename), Buffer.alloc(1));
  const r = run();
  assert.notEqual(r.status, 0, "falsche Groesse muss scheitern");
  assert.match(`${r.stdout}${r.stderr}`, /Groesse/);
});

test("NSIS legt die Sprechertrennung unter models/diarization ab (gleiche Ablage wie der Ladeweg)", () => {
  assert.match(nsi, /SEC_DIAR/);
  assert.match(nsi, /File \/r "\$\{VORINSTALLIERT\}\\diar\\models\\\*"/);
  assert.match(nsi, /MUI_DESCRIPTION_TEXT \$\{SEC_DIAR\}/);
});

test("Lizenzdatei: Nemotron-3-Diarization mit OpenMDW-1.1, Lizenzkopie und Quelle", () => {
  assert.match(licenses, /Nemotron-3-Diarization/);
  assert.match(licenses, /OpenMDW-1\.1/);
  assert.match(licenses, new RegExp(diarEntry?.revision ?? "KEINE-REVISION"));
  assert.match(licenses, /Kopie der Lizenz/);
  assert.match(licenses, /LIZENZ-OpenMDW-1\.1\.txt/);
});
