// Update-Quelle der App zur Bauzeit festlegen (wai-sdk ADR-0020, Beta-Kanal).
//
// Ohne Umgebungsvariablen bleibt alles beim Alten: tauri.conf.json zeigt auf die GitHub-Releases
// (Kanal "stable"). Fuer einen selbst betriebenen Update-Server (z. B. Beta-Kanal):
//
//   LVA_UPDATE_BASE=https://updates.example.org/wai    Basis-URL (HTTPS, ohne Schlusszeichen)
//   LVA_UPDATE_CHANNEL=nacht                           stable (Standard) oder nacht
//   LVA_UPDATE_PUBKEY=<Inhalt der .pub-Datei>          optional, sonst der Schluessel aus tauri.conf.json
//
// Ergebnis: JSON fuer `tauri build --config <Datei>`; der Endpunkt lautet
// <Basis>/<identifier>/<kanal>/latest.json (statische Datei im Format des Tauri-Updaters).
// Aufruf: node scripts/update-config-cli.mjs > update.conf.json   (leer = {} = keine Aenderung)
const KANAELE = new Set(["stable", "nacht"]);

export function buildConfig(env, identifier) {
  const base = (env.LVA_UPDATE_BASE ?? "").trim().replace(/\/+$/, "");
  const kanal = (env.LVA_UPDATE_CHANNEL ?? "").trim() || "stable";
  const pubkey = (env.LVA_UPDATE_PUBKEY ?? "").trim();
  if (!base && !pubkey) return {};
  if (!KANAELE.has(kanal)) throw new Error(`LVA_UPDATE_CHANNEL: stable oder nacht, nicht "${kanal}"`);
  const updater = {};
  if (base) {
    let url;
    try {
      url = new URL(base);
    } catch {
      throw new Error("LVA_UPDATE_BASE ist keine gueltige URL");
    }
    if (url.protocol !== "https:") throw new Error("LVA_UPDATE_BASE muss HTTPS sein");
    if (url.search || url.hash) throw new Error("LVA_UPDATE_BASE darf keine Query und kein Fragment enthalten");
    if (url.username || url.password) throw new Error("LVA_UPDATE_BASE darf keine Zugangsdaten enthalten");
    updater.endpoints = [`${base}/${identifier}/${kanal}/latest.json`];
  }
  if (pubkey) updater.pubkey = pubkey;
  return { plugins: { updater } };
}
