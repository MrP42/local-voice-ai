// Kommandozeile zu update-config.mjs (eigene Datei, damit es keine Pruefung "wurde ich direkt gestartet" gibt,
// die unter Windows an Laufwerksbuchstaben-Schreibweise oder Pfadform scheitern kann).
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { buildConfig } from "./update-config.mjs";

const dir = dirname(fileURLToPath(import.meta.url));
const conf = JSON.parse(readFileSync(join(dir, "..", "src-tauri", "tauri.conf.json"), "utf8"));
process.stdout.write(JSON.stringify(buildConfig(process.env, conf.identifier), null, 2) + "\n");
