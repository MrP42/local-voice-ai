// Lader für scripts/assistent-engine.test.mjs: löst den Alias `@/` auf, ergänzt `.ts` und ersetzt
// Tauri, Audio und Orchestrierung durch Attrappen (die Engine selbst läuft unverändert).
import { existsSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HIER = path.dirname(fileURLToPath(import.meta.url));
const SRC = path.resolve(HIER, "../../src");
const stub = (name) => path.join(HIER, name);

const PAKETE = {
  "@tauri-apps/api/core": stub("tauri-core.ts"),
  "@tauri-apps/api/event": stub("tauri-event.ts"),
  "@tauri-apps/api/webviewWindow": stub("leer.ts"),
};

const ERSATZ = new Map([
  [path.join(SRC, "components/voice-call/audio.ts"), stub("audio.ts")],
  [
    path.join(SRC, "components/voice-call/orchestration.ts"),
    stub("orchestration.ts"),
  ],
  [path.join(SRC, "stores/settingsStore.ts"), stub("settingsStore.ts")],
  [path.join(SRC, "stores/complianceStore.ts"), stub("complianceStore.ts")],
  [path.join(SRC, "i18n/index.ts"), stub("i18n.ts")],
]);

const ENDUNGEN = ["", ".ts", ".tsx", "/index.ts"];

function datei(basis) {
  // path.normalize: unter Windows mischt `basis + "/index.ts"` sonst Trennzeichen, und der
  // ERSATZ-Abgleich (z. B. i18n mit import.meta.glob) greift nicht.
  return ENDUNGEN.map((e) => path.normalize(basis + e)).find(
    (f) => existsSync(f) && statSync(f).isFile(),
  );
}

export async function resolve(specifier, context, next) {
  if (PAKETE[specifier])
    return { url: pathToFileURL(PAKETE[specifier]).href, shortCircuit: true };
  let basis = null;
  if (specifier.startsWith("@/")) basis = path.join(SRC, specifier.slice(2));
  else if (
    (specifier.startsWith("./") || specifier.startsWith("../")) &&
    context.parentURL?.startsWith("file:")
  )
    basis = path.resolve(
      path.dirname(fileURLToPath(context.parentURL)),
      specifier,
    );
  const gefunden = basis ? datei(basis) : undefined;
  if (!gefunden) return next(specifier, context);
  return {
    url: pathToFileURL(ERSATZ.get(gefunden) ?? gefunden).href,
    shortCircuit: true,
  };
}
