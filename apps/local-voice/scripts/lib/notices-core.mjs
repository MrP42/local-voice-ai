// Reine Logik fuer scripts/gen-notices.mjs (Issue #7): keine Prozesse, keine
// Dateizugriffe ausser ueber uebergebene Funktionen. Getestet in
// scripts/gen-notices.test.mjs.

const KNOWN_SPDX = [
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "CC-BY-4.0",
  "CC-BY-NC-4.0",
  "CC-BY-SA-4.0",
  "CC0-1.0",
  "GPL-3.0",
  "GPL-3.0-or-later",
  "ISC",
  "LGPL-3.0",
  "MIT",
  "MPL-2.0",
];

/** SPDX-Kennung am Anfang eines Lizenzfeldes, sonst null (Freitext, "other"). */
export function normalizeLicenseId(text) {
  if (typeof text !== "string") return null;
  const m = text.replace(/\*/g, "").trim().match(/^([A-Za-z0-9.+-]+)/);
  if (!m) return null;
  const hit = KNOWN_SPDX.find((id) => id.toLowerCase() === m[1].toLowerCase());
  return hit ?? null;
}

/**
 * Ganzer Lizenztext ist ein SPDX-Ausdruck mit AND/OR/WITH ("MIT AND GPL-3.0-or-later"),
 * sonst null. Solche Eintraege duerfen nicht auf die erste Kennung verkuerzt werden.
 */
export function licenseExpression(text) {
  if (typeof text !== "string") return null;
  const t = text.trim();
  return /^[A-Za-z0-9.+-]+(\s+(AND|OR|WITH)\s+[A-Za-z0-9.+-]+)+$/.test(t) ? t : null;
}

const firstUrl = (s) => (s.match(/<(https?:\/\/[^>]+)>/) ?? [])[1] ?? null;

/**
 * Liest docs/m2-evidence/ATTRIBUTION.md: Datensaetze (Ueberschrift
 * "## Name - LIZENZ") und die Tabelle unter "## Modelle".
 */
export function parseAttribution(md) {
  const lines = md.split(/\r?\n/);
  const datasets = [];
  for (let i = 0; i < lines.length; i++) {
    const h = lines[i].match(/^## (.+?) - ([A-Za-z0-9.+-]+)\s*$/);
    if (!h) continue;
    let source = null;
    for (let j = i + 1; j < lines.length && !lines[j].startsWith("## "); j++) {
      source = firstUrl(lines[j]);
      if (source) break;
    }
    datasets.push({ name: h[1], license: h[2], licenseId: normalizeLicenseId(h[2]), source });
  }

  const start = lines.findIndex((l) => /^## Modelle\s*$/.test(l));
  if (start < 0) throw new Error("ATTRIBUTION.md: Abschnitt '## Modelle' fehlt");
  const models = [];
  let seenSeparator = false;
  for (let i = start + 1; i < lines.length; i++) {
    const l = lines[i];
    if (l.startsWith("## ")) break;
    if (!l.trim().startsWith("|")) {
      if (models.length || seenSeparator) break;
      continue;
    }
    if (/^\|\s*-+/.test(l.trim())) {
      seenSeparator = true;
      continue;
    }
    if (!seenSeparator) continue; // Kopfzeile
    const cells = l.trim().replace(/^\||\|$/g, "").split("|").map((c) => c.trim());
    if (cells.length < 4) continue;
    const license = cells[2].replace(/\*\*/g, "");
    models.push({
      name: cells[0],
      use: cells[1],
      license,
      licenseId: normalizeLicenseId(license),
      source: firstUrl(cells[3]),
      bundled: /mitgeliefert/i.test(cells[1]),
    });
  }
  if (!models.length) throw new Error("ATTRIBUTION.md: Modelltabelle ohne Zeilen");
  return { datasets, models };
}

const normText = (t) => t.replace(/\r\n/g, "\n").trim();
const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

/**
 * pnpm licenses list --json -> flache Paketliste + Lizenztexte (gleiche Texte
 * zusammengefasst). readText(dir) liefert den Text der Lizenzdatei oder null.
 */
export function groupNpmLicenses(json, readText) {
  const packages = [];
  const byText = new Map();
  const missing = [];
  for (const [license, list] of Object.entries(json)) {
    for (const p of list) {
      p.versions.forEach((version, i) => {
        const id = `${p.name}@${version}`;
        packages.push({ name: p.name, version, license: p.license ?? license, homepage: p.homepage ?? null });
        const raw = readText(p.paths[i] ?? p.paths[0]);
        if (!raw || !raw.trim()) {
          missing.push(id);
          return;
        }
        const key = normText(raw);
        if (!byText.has(key)) byText.set(key, []);
        byText.get(key).push(id);
      });
    }
  }
  packages.sort((a, b) => cmp(a.name, b.name) || cmp(a.version, b.version));
  missing.sort();
  const texts = [...byText.entries()]
    .map(([text, pk]) => ({ text, packages: pk.sort() }))
    .sort((a, b) => cmp(a.packages[0], b.packages[0]));
  return { packages, texts, missing };
}

const PERMISSIVE_CATALOG = new Set(["MIT", "Apache-2.0", "CC-BY-4.0"]);

/** Modellkatalog (src/catalog/catalog.json) -> Liste mit Lizenzangaben. */
export function catalogModels(catalog) {
  return catalog.models
    .map((m) => {
      const license = typeof m.license === "string" && m.license.trim() ? m.license.trim() : null;
      const expression = licenseExpression(license);
      return {
        id: m.id,
        slug: m.slug,
        name: m.name,
        purpose: m.purpose ?? "asr",
        revision: m.revision ?? null,
        license,
        licenseExpression: expression,
        // Ein Ausdruck ("MIT AND GPL-3.0-or-later") ist keine einzelne freizuegige Kennung.
        licenseId: expression ? null : normalizeLicenseId(license),
        licenseUrl: typeof m.license_url === "string" && m.license_url.trim() ? m.license_url.trim() : null,
        licenseNote: typeof m.license_note === "string" && m.license_note.trim() ? m.license_note.trim() : null,
        nonCommercial: m.non_commercial === true || (license ? /(^|-)nc(-|$)/i.test(license) : false),
        files: (m.files ?? []).map((f) => ({ filename: f.filename, sha256: f.sha256 })),
      };
    })
    .sort((a, b) => cmp(a.slug ?? a.id, b.slug ?? b.id));
}

// ---------------------------------------------------------------- Notices

function fence(text) {
  const longest = Math.max(2, ...[...text.matchAll(/`+/g)].map((m) => m[0].length));
  const f = "`".repeat(longest + 1);
  return `${f}\n${text}\n${f}`;
}

export function renderNotices({ appName, version, about, npm, attribution, catalog }) {
  const out = [];
  out.push("# Third-Party-Notices");
  out.push("");
  out.push(
    `${appName} ${version} enthält Software und nutzt Modelle Dritter. Diese Datei nennt sie mit ihren Lizenzen und ` +
      "wird mit `apps/local-voice/scripts/gen-notices.mjs` aus `Cargo.lock`, `pnpm-lock.yaml`, dem Modellkatalog und " +
      "`docs/m2-evidence/ATTRIBUTION.md` erzeugt. Nicht von Hand ändern.",
  );
  out.push("");

  // Fund-Hinweise zuerst: wer die Datei liest, soll sie nicht suchen muessen.
  const copyleft = about.licenses.filter((l) => /^(A|L)?GPL/i.test(l.id));
  const restricted = catalog.filter((m) => !PERMISSIVE_CATALOG.has(m.licenseId));
  // Auch freizuegige Eintraege mit Herkunftshinweis (z. B. Piper-Stimme aus Lessac feinabgestimmt) nennen.
  const noted = catalog.filter((m) => m.licenseNote && PERMISSIVE_CATALOG.has(m.licenseId));
  out.push("## Hinweise");
  out.push("");
  if (copyleft.length) {
    out.push("**Bibliotheken unter Copyleft-Lizenz (Quelltext der Bibliothek ist über die genannte Adresse erhältlich):**");
    out.push("");
    for (const l of copyleft) {
      for (const u of l.used_by) {
        out.push(`- ${u.crate.name} ${u.crate.version} — ${l.id}${u.crate.repository ? ` — <${u.crate.repository}>` : ""}`);
      }
    }
    out.push("");
  }
  const catalogCopyleft = catalog.filter((m) => /(^|[\s(])(A|L)?GPL-/i.test(m.license ?? ""));
  if (catalogCopyleft.length) {
    out.push("**Heruntergeladene Laufzeiten mit Copyleft-Anteil (nicht im Installer, auf Wunsch von der Quelle geladen):**");
    out.push("");
    for (const m of catalogCopyleft) out.push(`- ${m.name} — ${m.license}${m.licenseUrl ? ` — <${m.licenseUrl}>` : ""}`);
    out.push("");
  }
  const nc = restricted.filter((m) => m.nonCommercial);
  if (nc.length) {
    out.push("**Modelle mit nicht-kommerzieller Lizenz (im Katalog wählbar, nicht vorgewählt):**");
    out.push("");
    for (const m of nc) out.push(`- ${m.name} — ${m.licenseId ?? m.license}`);
    out.push("");
  }
  out.push(
    "Modelle werden bei Bedarf geladen und nicht mit dem Installer ausgeliefert; Ausnahme ist die kleine Silero-VAD-Datei.",
  );
  out.push("");

  out.push("## Modelle");
  out.push("");
  out.push("| Modell | Verwendung | Lizenz | Quelle |");
  out.push("|---|---|---|---|");
  for (const m of attribution.models) {
    out.push(`| ${m.name} | ${m.use} | ${m.license} | ${m.source ? `<${m.source}>` : ""} |`);
  }
  out.push("");
  const explicit = [...restricted.filter((m) => m.license), ...noted].sort((a, b) => cmp(a.slug ?? a.id, b.slug ?? b.id));
  if (explicit.length) {
    out.push(
      "Weitere Katalogeinträge (Modelle, Stimmen, Laufzeiten) mit eigener, gemischter oder nicht freizügiger Lizenz bzw. Herkunftshinweis (maßgeblich ist die Quelle):",
    );
    out.push("");
    for (const m of explicit) {
      const flag = m.nonCommercial ? " — **nur nicht-kommerziell**" : "";
      const url = m.licenseUrl ? ` — <${m.licenseUrl}>` : "";
      const note = m.licenseNote ? `. ${m.licenseNote}` : "";
      out.push(`- ${m.name} (\`${m.id}\`) — ${m.license}${flag}${url}${note}`);
    }
    out.push("");
  }
  const unspecified = restricted.filter((m) => !m.license);
  if (unspecified.length) {
    out.push("Katalogeinträge ohne Lizenzangabe im Katalog (Lizenz laut Quelle bzw. Modellkarte, Download-Adresse im Katalog):");
    out.push("");
    const byPurpose = new Map();
    for (const m of unspecified) {
      if (!byPurpose.has(m.purpose)) byPurpose.set(m.purpose, []);
      byPurpose.get(m.purpose).push(m.name);
    }
    for (const [purpose, names] of [...byPurpose].sort((a, b) => cmp(a[0], b[0]))) {
      out.push(`- ${purpose}: ${names.join("; ")}`);
    }
    out.push("");
  }

  out.push("## Datensätze (nur Messungen, nicht im Installer)");
  out.push("");
  for (const d of attribution.datasets) {
    out.push(`- ${d.name} — ${d.license}${d.source ? ` — <${d.source}>` : ""}`);
  }
  out.push("");

  // Rust
  const crateIndex = new Map();
  for (const l of about.licenses) {
    for (const u of l.used_by) {
      const key = `${u.crate.name} ${u.crate.version}`;
      if (!crateIndex.has(key)) crateIndex.set(key, new Set());
      crateIndex.get(key).add(l.id);
    }
  }
  const crateKeys = [...crateIndex.keys()].sort(cmp);
  out.push(`## Rust-Bibliotheken (${crateKeys.length})`);
  out.push("");
  for (const k of crateKeys) out.push(`- ${k} — ${[...crateIndex.get(k)].sort().join(", ")}`);
  out.push("");
  out.push("### Lizenztexte (Rust)");
  out.push("");
  const rustTexts = [...about.licenses]
    .map((l) => ({
      ...l,
      crates: l.used_by.map((u) => `${u.crate.name} ${u.crate.version}`).sort(cmp),
    }))
    .sort((a, b) => cmp(a.id, b.id) || cmp(a.crates[0], b.crates[0]));
  for (const l of rustTexts) {
    out.push(`#### ${l.name} (${l.id}) — ${l.crates.join(", ")}`);
    out.push("");
    out.push(fence(normText(l.text)));
    out.push("");
  }

  // npm
  out.push(`## Frontend-Pakete (npm, Produktionsabhängigkeiten, ${npm.packages.length})`);
  out.push("");
  for (const p of npm.packages) out.push(`- ${p.name} ${p.version} — ${p.license}`);
  out.push("");
  if (npm.missing.length) {
    out.push(`Ohne Lizenzdatei im Paket (Lizenz laut package.json, plattformspezifische Binärpakete): ${npm.missing.join(", ")}`);
    out.push("");
  }
  out.push("### Lizenztexte (npm)");
  out.push("");
  for (const t of npm.texts) {
    out.push(`#### ${t.packages.join(", ")}`);
    out.push("");
    out.push(fence(t.text));
    out.push("");
  }
  return out.join("\n").replace(/\n+$/, "\n");
}

// ------------------------------------------------------------------ SBOM

const slugify = (s) => s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");

function licenseEntries(license, licenseId) {
  if (licenseId) return [{ license: { id: licenseId } }];
  if (license) return [{ license: { name: license } }];
  return undefined;
}

function npmLicenses(expr) {
  if (!expr || /^unknown$/i.test(expr)) return undefined;
  if (/\s(OR|AND|WITH)\s/.test(expr)) return [{ expression: expr }];
  if (/^[A-Za-z0-9.+-]+$/.test(expr)) return [{ license: { id: expr } }];
  return [{ license: { name: expr } }];
}

const npmPurl = (name, version) =>
  `pkg:npm/${name.startsWith("@") ? `%40${name.slice(1)}` : name}@${version}`;

/**
 * Haengt npm-Pakete, Modelle und Datensaetze an die Rust-SBOM von
 * cargo-cyclonedx an (CycloneDX 1.5).
 */
export function buildSbom({ cargoBom, npm, attribution, catalog }) {
  const bom = structuredClone(cargoBom);
  bom.components ??= [];
  bom.dependencies ??= [];
  const shipped = [];

  const seenNpm = new Set();
  const addNpm = (p, scope) => {
    const ref = npmPurl(p.name, p.version);
    if (seenNpm.has(ref)) return;
    seenNpm.add(ref);
    const c = { type: "library", "bom-ref": ref, name: p.name, version: p.version, scope, purl: ref };
    const lic = npmLicenses(p.license);
    if (lic) c.licenses = lic;
    bom.components.push(c);
    if (scope === "required") shipped.push(ref);
  };
  for (const p of npm.prod) addNpm(p, "required");
  for (const p of npm.dev) addNpm(p, "excluded");

  for (const m of attribution.models) {
    const ref = `model:attribution:${slugify(m.name)}`;
    const c = {
      type: "machine-learning-model",
      "bom-ref": ref,
      name: m.name,
      description: m.use,
      scope: m.bundled ? "required" : "optional",
      properties: [
        { name: "lva:license-text", value: m.license },
        { name: "lva:origin", value: "docs/m2-evidence/ATTRIBUTION.md" },
      ],
    };
    const lic = licenseEntries(m.license, m.licenseId);
    if (lic) c.licenses = lic;
    if (m.source) c.externalReferences = [{ type: "website", url: m.source }];
    bom.components.push(c);
    shipped.push(ref);
  }

  for (const d of attribution.datasets) {
    const ref = `data:${slugify(d.name)}`;
    const c = {
      type: "data",
      "bom-ref": ref,
      name: d.name,
      scope: "excluded",
      properties: [{ name: "lva:origin", value: "docs/m2-evidence/ATTRIBUTION.md" }],
    };
    const lic = licenseEntries(d.license, d.licenseId);
    if (lic) c.licenses = lic;
    if (d.source) c.externalReferences = [{ type: "website", url: d.source }];
    bom.components.push(c);
  }

  for (const m of catalog) {
    const ref = `model:catalog:${slugify(m.slug ?? m.id)}`;
    const c = {
      type: /runtime$/.test(m.purpose) ? "application" : "machine-learning-model",
      "bom-ref": ref,
      name: m.name,
      scope: "optional",
      properties: [
        { name: "lva:origin", value: "src-tauri/src/catalog/catalog.json" },
        { name: "lva:license-text", value: m.license ?? "keine Angabe im Katalog" },
      ],
    };
    if (m.revision) c.version = m.revision;
    if (m.id && m.revision) c.purl = `pkg:huggingface/${m.id}@${m.revision}`;
    if (m.nonCommercial) c.properties.push({ name: "lva:non-commercial", value: "true" });
    for (const f of m.files) c.properties.push({ name: `lva:file:${f.filename}`, value: `sha256:${f.sha256}` });
    if (m.licenseNote) c.properties.push({ name: "lva:license-note", value: m.licenseNote });
    const lic = m.licenseExpression ? [{ expression: m.licenseExpression }] : licenseEntries(m.license, m.licenseId);
    if (lic) c.licenses = lic;
    if (m.licenseUrl) c.externalReferences = [{ type: "license", url: m.licenseUrl }];
    bom.components.push(c);
    shipped.push(ref);
  }

  const rootRef = bom.metadata?.component?.["bom-ref"];
  if (rootRef) {
    let root = bom.dependencies.find((d) => d.ref === rootRef);
    if (!root) {
      root = { ref: rootRef, dependsOn: [] };
      bom.dependencies.unshift(root);
    }
    root.dependsOn = [...new Set([...(root.dependsOn ?? []), ...shipped])];
  }
  return bom;
}

/** Strukturpruefung: Format, eindeutige bom-refs, aufloesbare Abhaengigkeiten. */
export function validateBom(bom) {
  const problems = [];
  if (bom.bomFormat !== "CycloneDX") problems.push("bomFormat ist nicht CycloneDX");
  if (!bom.specVersion) problems.push("specVersion fehlt");
  const refs = new Set();
  const all = [bom.metadata?.component, ...(bom.components ?? [])].filter(Boolean);
  for (const c of all) {
    const ref = c["bom-ref"];
    if (!ref) continue;
    if (refs.has(ref)) problems.push(`bom-ref doppelt: ${ref}`);
    refs.add(ref);
  }
  for (const d of bom.dependencies ?? []) {
    if (!refs.has(d.ref)) problems.push(`Abhängigkeit mit unbekanntem ref: ${d.ref}`);
    for (const t of d.dependsOn ?? []) if (!refs.has(t)) problems.push(`Abhängigkeit auf unbekanntes Ziel: ${t} (von ${d.ref})`);
  }
  return problems;
}
