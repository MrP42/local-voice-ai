#!/usr/bin/env node
// Third-Party-Notices und SBOM erzeugen (Issue #7).
//
//   node apps/local-voice/scripts/gen-notices.mjs                 # beides
//   node apps/local-voice/scripts/gen-notices.mjs --only notices  # nur Notices
//   node apps/local-voice/scripts/gen-notices.mjs --only sbom     # nur SBOM
//   node apps/local-voice/scripts/gen-notices.mjs --check         # Notices aktuell?
//   pwsh -File apps\local-voice\scripts\dev.ps1 notices           # Wrapper
//
// Ergebnis:
//   src-tauri/resources/THIRD-PARTY-NOTICES.md   im Repo und (resources/**) im Installer
//   src-tauri/target/sbom/local-voice-ai-<version>.cdx.json   CycloneDX 1.5, Build-Artefakt
//
// Quellen: cargo-about (Rust, about.toml), pnpm licenses (npm), Modellkatalog
// und docs/m2-evidence/ATTRIBUTION.md (Modelle, Datensaetze). cargo-about und
// cargo-cyclonedx sind reine Entwicklerwerkzeuge (MIT OR Apache-2.0 bzw.
// Apache-2.0), nichts davon landet im Produkt.
//
// Exit 2 = Werkzeug fehlt (mit Installationshinweis), 1 = Schritt fehlgeschlagen
// oder (--check) Notices veraltet. Jeder Schritt prueft seinen eigenen Exit-Code.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  buildSbom,
  catalogModels,
  groupNpmLicenses,
  parseAttribution,
  renderNotices,
  validateBom,
} from "./lib/notices-core.mjs";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const appDir = path.resolve(scriptDir, "..");
const tauriDir = path.join(appDir, "src-tauri");
const repoRoot = path.resolve(appDir, "..", "..");
const noticesPath = path.join(tauriDir, "resources", "THIRD-PARTY-NOTICES.md");
const attributionPath = path.join(repoRoot, "docs", "m2-evidence", "ATTRIBUTION.md");
const catalogPath = path.join(tauriDir, "src", "catalog", "catalog.json");

// ------------------------------------------------------------------ Argumente
const args = process.argv.slice(2);
const opt = (name, fallback = null) => {
  const i = args.indexOf(name);
  return i >= 0 ? (args[i + 1] ?? fallback) : fallback;
};
const only = opt("--only"); // notices | sbom | null
const check = args.includes("--check");
const outDir = path.resolve(opt("--out-dir", path.join(tauriDir, "target", "sbom")));
const sbomTarget = opt("--sbom-target", "x86_64-pc-windows-msvc");
if (only && !["notices", "sbom"].includes(only)) {
  console.error("FEHLER: --only erwartet 'notices' oder 'sbom'.");
  process.exit(1);
}

// ----------------------------------------------------------------------- PATH
// cargo liegt unter ~/.cargo/bin und ist in Git Bash/PowerShell oft nicht im
// PATH (Issue #8). Hier selbst ergaenzen statt "command not found" zu riskieren.
const pathKey = Object.keys(process.env).find((k) => k.toLowerCase() === "path") ?? "PATH";
const cargoBin = path.join(os.homedir(), ".cargo", "bin");
if (fs.existsSync(cargoBin) && !process.env[pathKey].split(path.delimiter).includes(cargoBin)) {
  process.env[pathKey] = cargoBin + path.delimiter + process.env[pathKey];
}

function run(cmd, cmdArgs, options = {}) {
  const base = { encoding: "utf8", maxBuffer: 512 * 1024 * 1024, ...options };
  // pnpm ist unter Windows ein .cmd und braucht die Shell. Als fertige
  // Kommandozeile uebergeben (feste Argumente), sonst warnt Node (DEP0190).
  if (cmd === "pnpm" && process.platform === "win32") {
    return spawnSync([cmd, ...cmdArgs].join(" "), { ...base, shell: true });
  }
  return spawnSync(cmd, cmdArgs, base);
}

function need(cmd, cmdArgs, hint) {
  const r = run(cmd, cmdArgs);
  if (r.error || r.status !== 0) {
    console.error(`FEHLT: ${[cmd, ...cmdArgs].join(" ")} schlägt fehl. ${hint}`);
    process.exit(2);
  }
  return r.stdout.trim().split(/\r?\n/)[0];
}

function step(label, r) {
  if (r.error || r.status !== 0) {
    console.error(`FEHLGESCHLAGEN: ${label} (Exit ${r.status ?? r.error?.code})`);
    if (r.stderr) console.error(r.stderr.split(/\r?\n/).slice(-15).join("\n"));
    process.exit(1);
  }
  console.log(`OK: ${label}`);
}

const readJson = (p) => JSON.parse(fs.readFileSync(p, "utf8"));

// ------------------------------------------------------------------------ npm
function npmLists() {
  need("pnpm", ["--version"], "pnpm installieren (npm i -g pnpm).");
  if (!fs.existsSync(path.join(appDir, "node_modules"))) {
    console.error(
      "FEHLT: node_modules. Zuerst im App-Verzeichnis: pnpm install --frozen-lockfile (pnpm licenses liest den installierten Baum).",
    );
    process.exit(2);
  }
  const prod = run("pnpm", ["licenses", "list", "--prod", "--json"], { cwd: appDir });
  step("pnpm licenses list --prod", prod);
  const all = run("pnpm", ["licenses", "list", "--json"], { cwd: appDir });
  step("pnpm licenses list", all);
  return { prodJson: JSON.parse(prod.stdout), allJson: JSON.parse(all.stdout) };
}

function readLicenseText(dir) {
  if (!dir || !fs.existsSync(dir)) return null;
  const files = fs
    .readdirSync(dir)
    .filter((f) => /^(licen[sc]e|copying)/i.test(f) && fs.statSync(path.join(dir, f)).isFile())
    .sort();
  if (!files.length) return null;
  return files.map((f) => fs.readFileSync(path.join(dir, f), "utf8")).join("\n\n");
}

function flatten(json) {
  const out = [];
  for (const [license, list] of Object.entries(json)) {
    for (const p of list) for (const version of p.versions) out.push({ name: p.name, version, license: p.license ?? license });
  }
  return out;
}

// ----------------------------------------------------------------------- Rust
function cargoAbout(tmp) {
  need("cargo", ["--version"], "Rust installieren; cargo liegt unter %USERPROFILE%\\.cargo\\bin.");
  need(
    "cargo",
    ["about", "--version"],
    "Installieren: cargo install cargo-about --locked --features cli (MIT OR Apache-2.0, nur Entwicklerwerkzeug).",
  );
  const file = path.join(tmp, "about.json");
  const r = run(
    "cargo",
    ["about", "generate", "--format", "json", "--locked", "--manifest-path", path.join(tauriDir, "Cargo.toml"), "-o", file],
    { cwd: tauriDir },
  );
  step("cargo about generate", r);
  return readJson(file);
}

function cargoCycloneDx(tmp) {
  need(
    "cargo",
    ["cyclonedx", "--version"],
    "Installieren: cargo install cargo-cyclonedx --locked (Apache-2.0, nur Entwicklerwerkzeug).",
  );
  const name = "g-sbom-rust";
  const produced = path.join(tauriDir, `${name}.json`);
  const r = run(
    "cargo",
    [
      "cyclonedx",
      "--manifest-path",
      path.join(tauriDir, "Cargo.toml"),
      "--format",
      "json",
      "--spec-version",
      "1.5",
      "--target",
      sbomTarget,
      "--override-filename",
      name,
    ],
    { cwd: tauriDir },
  );
  step(`cargo cyclonedx (${sbomTarget})`, r);
  if (!fs.existsSync(produced)) {
    console.error(`FEHLGESCHLAGEN: cargo cyclonedx hat ${produced} nicht erzeugt.`);
    process.exit(1);
  }
  const bom = readJson(produced);
  fs.rmSync(produced); // cargo-cyclonedx schreibt neben die Cargo.toml; nichts liegen lassen
  return bom;
}

// ----------------------------------------------------------------------- main
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "lva-notices-"));
try {
  const version = readJson(path.join(tauriDir, "tauri.conf.json")).version;
  const attribution = parseAttribution(fs.readFileSync(attributionPath, "utf8"));
  const catalog = catalogModels(readJson(catalogPath));
  const { prodJson, allJson } = npmLists();

  if (only !== "sbom") {
    const about = cargoAbout(tmp);
    const npm = groupNpmLicenses(prodJson, readLicenseText);
    const text = renderNotices({ appName: "Local Voice AI", version, about, npm, attribution, catalog });
    if (check) {
      const current = fs.existsSync(noticesPath) ? fs.readFileSync(noticesPath, "utf8") : "";
      if (current.replace(/\r\n/g, "\n") !== text) {
        console.error(
          "VERALTET: resources/THIRD-PARTY-NOTICES.md passt nicht zu Cargo.lock/pnpm-lock/ATTRIBUTION.md. Neu erzeugen: node apps/local-voice/scripts/gen-notices.mjs --only notices",
        );
        process.exit(1);
      }
      console.log("OK: THIRD-PARTY-NOTICES.md ist aktuell");
    } else {
      fs.mkdirSync(path.dirname(noticesPath), { recursive: true });
      fs.writeFileSync(noticesPath, text, "utf8");
      console.log(`OK: ${path.relative(repoRoot, noticesPath)} (${(text.length / 1024).toFixed(0)} KiB)`);
    }
  }

  if (only !== "notices" && !check) {
    const prodKeys = new Set(flatten(prodJson).map((p) => `${p.name}@${p.version}`));
    const npm = {
      prod: flatten(prodJson),
      dev: flatten(allJson).filter((p) => !prodKeys.has(`${p.name}@${p.version}`)),
    };
    const bom = buildSbom({ cargoBom: cargoCycloneDx(tmp), npm, attribution, catalog });
    const problems = validateBom(bom);
    if (problems.length) {
      console.error("FEHLGESCHLAGEN: SBOM ist nicht stimmig:\n- " + problems.slice(0, 20).join("\n- "));
      process.exit(1);
    }
    fs.mkdirSync(outDir, { recursive: true });
    const file = path.join(outDir, `local-voice-ai-${version}.cdx.json`);
    fs.writeFileSync(file, JSON.stringify(bom, null, 2) + "\n", "utf8");
    console.log(`OK: ${file} (${bom.components.length} Komponenten)`);
  }
} finally {
  fs.rmSync(tmp, { recursive: true, force: true });
}
