// Tests fuer scripts/lib/notices-core.mjs (Issue #7). Lauf:
//   node --test apps/local-voice/scripts/gen-notices.test.mjs
// Reine Logik, kein cargo und kein pnpm noetig.
import test from "node:test";
import assert from "node:assert/strict";
import {
  licenseExpression,
  normalizeLicenseId,
  parseAttribution,
  groupNpmLicenses,
  renderNotices,
  catalogModels,
  buildSbom,
  validateBom,
} from "./lib/notices-core.mjs";

const ATTRIBUTION = `# Namensnennung

## FLEURS (Google) - CC-BY-4.0

- Quelle: <https://huggingface.co/datasets/google/fleurs>
- Lizenz: Creative Commons Namensnennung 4.0 International (CC-BY-4.0).

## Synthetischer Mehrsprecher-Korpus

Keine Fremdlizenz.

## AMI Meeting Corpus - CC-BY-4.0

- Quelle: <https://groups.inf.ed.ac.uk/ami/corpus/>; bezogen ueber den Datensatz

## Modelle

| Modell | Verwendung | Lizenz | Quelle |
|---|---|---|---|
| Parakeet-TDT-0.6B-v3 (NVIDIA) | Live-Transkript | CC-BY-4.0, Namensnennung NVIDIA | <https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3>; GGUF |
| Silero VAD v4 (Silero Team) | VAD, \`silero_vad_v4.onnx\` mitgeliefert | MIT | <https://github.com/snakers4/silero-vad> |
| NVIDIA Streaming Sortformer | Sprechertrennung | **NVIDIA Open Model License** (kein Standardtext) | <https://huggingface.co/nvidia/diar> |
| Gemma 3 4B (Google) | LLM | Gemma Terms of Use (eigene Lizenz) | <https://huggingface.co/google/gemma-3-4b-it> |

**Sortformer:** Fliesstext nach der Tabelle.
`;

test("normalizeLicenseId erkennt SPDX am Anfang, sonst null", () => {
  assert.equal(normalizeLicenseId("CC-BY-4.0, Namensnennung NVIDIA"), "CC-BY-4.0");
  assert.equal(normalizeLicenseId("MIT"), "MIT");
  assert.equal(normalizeLicenseId("Apache-2.0 laut Modellkarte (MIT)"), "Apache-2.0");
  assert.equal(normalizeLicenseId("cc-by-nc-4.0"), "CC-BY-NC-4.0");
  assert.equal(normalizeLicenseId("**NVIDIA Open Model License** (kein Standardtext)"), null);
  assert.equal(normalizeLicenseId("Gemma Terms of Use"), null);
  assert.equal(normalizeLicenseId(undefined), null);
  assert.equal(normalizeLicenseId("other"), null);
});

test("parseAttribution liest Datensaetze (Ueberschrift mit Lizenz) und Modelltabelle", () => {
  const a = parseAttribution(ATTRIBUTION);
  assert.deepEqual(
    a.datasets.map((d) => [d.name, d.licenseId, d.source]),
    [
      ["FLEURS (Google)", "CC-BY-4.0", "https://huggingface.co/datasets/google/fleurs"],
      ["AMI Meeting Corpus", "CC-BY-4.0", "https://groups.inf.ed.ac.uk/ami/corpus/"],
    ],
  );
  assert.equal(a.models.length, 4);
  const [parakeet, silero, sortformer, gemma] = a.models;
  assert.equal(parakeet.name, "Parakeet-TDT-0.6B-v3 (NVIDIA)");
  assert.equal(parakeet.licenseId, "CC-BY-4.0");
  assert.equal(parakeet.source, "https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3");
  assert.equal(parakeet.bundled, false);
  assert.equal(silero.bundled, true, "Silero wird mitgeliefert");
  assert.equal(sortformer.licenseId, null);
  assert.equal(sortformer.license, "NVIDIA Open Model License (kein Standardtext)");
  assert.equal(gemma.licenseId, null);
});

test("parseAttribution scheitert laut, wenn die Modelltabelle fehlt", () => {
  assert.throws(() => parseAttribution("# nichts\n"), /Modelle/);
});

test("groupNpmLicenses fasst gleiche Lizenztexte zusammen und meldet fehlende Dateien", () => {
  const json = {
    MIT: [
      { name: "a", versions: ["1.0.0"], paths: ["/p/a"], license: "MIT", homepage: "h" },
      { name: "@s/b", versions: ["2.0.0"], paths: ["/p/b"], license: "MIT" },
      { name: "nolic", versions: ["3.0.0"], paths: ["/p/n"], license: "MIT" },
    ],
    ISC: [{ name: "c", versions: ["1.1.0", "1.2.0"], paths: ["/p/c1", "/p/c2"], license: "ISC" }],
  };
  const files = {
    "/p/a": "MIT License\n\nCopyright (c) A",
    "/p/b": "MIT License\n\nCopyright (c) A\n",
    "/p/c1": "ISC text",
    "/p/c2": "ISC text",
  };
  const g = groupNpmLicenses(json, (dir) => files[dir] ?? null);
  assert.deepEqual(
    g.packages.map((p) => `${p.name}@${p.version}`),
    ["@s/b@2.0.0", "a@1.0.0", "c@1.1.0", "c@1.2.0", "nolic@3.0.0"],
  );
  // a und @s/b teilen den Text (Zeilenende/Leerraum am Ende zaehlt nicht)
  const mit = g.texts.find((t) => t.text.startsWith("MIT License"));
  assert.deepEqual(mit.packages, ["@s/b@2.0.0", "a@1.0.0"]);
  assert.deepEqual(g.missing, ["nolic@3.0.0"]);
});

test("catalogModels uebernimmt Lizenz, Revision und Dateipruefsummen", () => {
  const cat = {
    models: [
      {
        id: "o/x-gguf",
        slug: "x",
        name: "X",
        revision: "abc",
        license: "cc-by-nc-4.0",
        files: [{ filename: "x-Q4.gguf", sha256: "00ff" }],
      },
      { id: "o/y-gguf", slug: "y", name: "Y", revision: "def", files: [] },
    ],
  };
  const m = catalogModels(cat);
  assert.equal(m[0].licenseId, "CC-BY-NC-4.0");
  assert.equal(m[0].nonCommercial, true);
  assert.deepEqual(m[0].files, [{ filename: "x-Q4.gguf", sha256: "00ff" }]);
  assert.equal(m[1].licenseId, null);
  assert.equal(m[1].license, null);
  assert.equal(m[0].purpose, "asr", "Standardzweck ohne purpose-Feld");
});

const cargoBom = () => ({
  bomFormat: "CycloneDX",
  specVersion: "1.5",
  version: 1,
  metadata: { component: { type: "application", "bom-ref": "root", name: "local-voice-ai", version: "1.0.0" } },
  components: [
    { type: "library", "bom-ref": "pkg:cargo/serde@1.0.0", name: "serde", version: "1.0.0" },
  ],
  dependencies: [
    { ref: "root", dependsOn: ["pkg:cargo/serde@1.0.0"] },
    { ref: "pkg:cargo/serde@1.0.0", dependsOn: [] },
  ],
});

test("buildSbom haengt npm, Modelle und Datensaetze an und besteht validateBom", () => {
  const a = parseAttribution(ATTRIBUTION);
  const npm = {
    prod: [{ name: "@s/b", version: "2.0.0", license: "MIT" }],
    dev: [{ name: "vite", version: "6.0.0", license: "MIT OR Apache-2.0" }],
  };
  const cat = catalogModels({
    models: [{ id: "o/x-gguf", slug: "x", name: "X", revision: "abc", license: "cc-by-nc-4.0", files: [{ filename: "f", sha256: "00" }] }],
  });
  const bom = buildSbom({ cargoBom: cargoBom(), npm, attribution: a, catalog: cat });
  assert.deepEqual(validateBom(bom), []);
  const by = (n) => bom.components.find((c) => c.name === n);
  assert.equal(by("@s/b").purl, "pkg:npm/%40s/b@2.0.0");
  assert.equal(by("@s/b").scope, "required");
  assert.equal(by("vite").scope, "excluded");
  assert.deepEqual(by("vite").licenses, [{ expression: "MIT OR Apache-2.0" }]);
  assert.equal(by("Parakeet-TDT-0.6B-v3 (NVIDIA)").type, "machine-learning-model");
  assert.equal(by("Parakeet-TDT-0.6B-v3 (NVIDIA)").scope, "optional");
  assert.equal(by("Silero VAD v4 (Silero Team)").scope, "required");
  assert.equal(by("FLEURS (Google)").type, "data");
  assert.equal(by("FLEURS (Google)").scope, "excluded");
  assert.equal(by("X").licenses[0].license.id, "CC-BY-NC-4.0");
  const root = bom.dependencies.find((d) => d.ref === "root");
  assert.ok(root.dependsOn.includes("pkg:npm/%40s/b@2.0.0"));
  assert.ok(!root.dependsOn.includes("pkg:npm/vite@6.0.0"), "Dev-Werkzeuge sind keine Abhaengigkeit des Produkts");
});

test("validateBom findet doppelte bom-refs und haengende Abhaengigkeiten", () => {
  const bom = cargoBom();
  bom.components.push({ ...bom.components[0] });
  bom.dependencies.push({ ref: "nirgends", dependsOn: ["auch-nicht"] });
  const problems = validateBom(bom);
  assert.ok(problems.some((p) => /doppelt/.test(p)));
  assert.ok(problems.some((p) => /nirgends/.test(p)));
  assert.ok(problems.some((p) => /auch-nicht/.test(p)));
});

test("renderNotices nennt LGPL-Fund, Canary und alle Teile, ohne Zeitstempel", () => {
  const about = {
    licenses: [
      {
        name: "GNU Lesser General Public License v3.0 only",
        id: "LGPL-3.0",
        text: "GNU LESSER GENERAL PUBLIC LICENSE\nVersion 3",
        used_by: [{ crate: { name: "mp3lame-encoder", version: "0.2.5", repository: "https://github.com/DoumanAsh/mp3lame-encoder" } }],
      },
      {
        name: "MIT License",
        id: "MIT",
        text: "MIT License\nCopyright (c) Serde",
        used_by: [{ crate: { name: "serde", version: "1.0.0", repository: null } }],
      },
    ],
  };
  const npm = groupNpmLicenses(
    { MIT: [{ name: "react", versions: ["18.3.1"], paths: ["/r"], license: "MIT" }] },
    () => "MIT License\nCopyright (c) Meta",
  );
  const cat = catalogModels({
    models: [{ id: "o/canary", slug: "canary", name: "Canary 1B", revision: "r", license: "cc-by-nc-4.0", files: [] }],
  });
  const text = renderNotices({
    appName: "Local Voice AI",
    version: "1.2.3",
    about,
    npm,
    attribution: parseAttribution(ATTRIBUTION),
    catalog: cat,
  });
  assert.match(text, /^# Third-Party-Notices/m);
  assert.match(text, /Local Voice AI 1\.2\.3/);
  assert.match(text, /mp3lame-encoder 0\.2\.5/);
  assert.match(text, /LGPL-3\.0/);
  assert.match(text, /GNU LESSER GENERAL PUBLIC LICENSE/);
  assert.match(text, /Canary 1B/);
  assert.match(text, /CC-BY-NC-4\.0/);
  assert.match(text, /Parakeet-TDT-0\.6B-v3/);
  assert.match(text, /FLEURS/);
  assert.match(text, /react 18\.3\.1/);
  assert.equal(renderNotices({ appName: "Local Voice AI", version: "1.2.3", about, npm, attribution: parseAttribution(ATTRIBUTION), catalog: cat }), text, "deterministisch");
});

test("licenseExpression erkennt SPDX-Ausdruecke, nicht Freitext", () => {
  assert.equal(licenseExpression("MIT AND GPL-3.0-or-later"), "MIT AND GPL-3.0-or-later");
  assert.equal(licenseExpression("MIT AND Apache-2.0 WITH LLVM-exception"), "MIT AND Apache-2.0 WITH LLVM-exception");
  assert.equal(licenseExpression("MIT"), null);
  assert.equal(licenseExpression("Gemma Terms of Use"), null);
  assert.equal(licenseExpression("MIT; CUDA-Bibliotheken: NVIDIA CUDA Toolkit EULA"), null);
});

test("Katalog: Ausdruck, URL, Hinweis und non_commercial landen in Notices und SBOM", () => {
  const cat = catalogModels({
    models: [
      {
        id: "piper-runtime-windows-x64",
        name: "Piper Runtime",
        purpose: "tts-runtime",
        license: "MIT AND GPL-3.0-or-later",
        license_url: "https://example.org/piper",
        license_note: "enthaelt espeak-ng",
        files: [],
      },
      { id: "en_US-lessac-medium", name: "Lessac", purpose: "tts-voice", license: "Blizzard-2013-Research-Licence", non_commercial: true, files: [] },
      { id: "de_DE-thorsten-high", name: "Thorsten", purpose: "tts-voice", license: "CC0-1.0", license_note: "aus Lessac feinabgestimmt", files: [] },
      { id: "o/y-gguf", name: "Y", license: "Apache-2.0", files: [] },
    ],
  });
  assert.equal(cat.find((m) => m.id === "piper-runtime-windows-x64").licenseId, null, "Ausdruck ist keine einzelne freizuegige Kennung");
  assert.equal(cat.find((m) => m.id === "piper-runtime-windows-x64").licenseExpression, "MIT AND GPL-3.0-or-later");
  assert.equal(cat.find((m) => m.id === "en_US-lessac-medium").nonCommercial, true);
  const about = { licenses: [] };
  const npm = groupNpmLicenses({}, () => null);
  const text = renderNotices({ appName: "Local Voice AI", version: "1", about, npm, attribution: parseAttribution(ATTRIBUTION), catalog: cat });
  assert.match(text, /Piper Runtime.*MIT AND GPL-3\.0-or-later.*<https:\/\/example\.org\/piper>\. enthaelt espeak-ng/);
  assert.match(text, /Lessac.*nur nicht-kommerziell/);
  assert.match(text, /Thorsten.*aus Lessac feinabgestimmt/);
  assert.doesNotMatch(text, /\(`o\/y-gguf`\)/, "freizuegig und ohne Hinweis: nicht gelistet");
  const bom = buildSbom({
    cargoBom: cargoBom(),
    npm: { prod: [], dev: [] },
    attribution: parseAttribution(ATTRIBUTION),
    catalog: cat,
  });
  const piper = bom.components.find((c) => c.name === "Piper Runtime");
  assert.equal(piper.type, "application");
  assert.deepEqual(piper.licenses, [{ expression: "MIT AND GPL-3.0-or-later" }]);
  assert.deepEqual(piper.externalReferences, [{ type: "license", url: "https://example.org/piper" }]);
  assert.deepEqual(validateBom(bom), []);
});
