import { type } from "@tauri-apps/plugin-os";

/**
 * Ob die App Zugangsdaten sicher speichern kann. Der Tresor
 * (`calendar::secret`) verschluesselt unter Windows mit DPAPI und unter macOS
 * mit einem Schluessel im Schluesselbund (lva#134); unter Linux lehnt er jedes
 * Speichern ab, ohne Klartext-Ersatz. Die Oberflaeche sperrt deshalb dort alles,
 * was ein Geheimnis braucht, und sagt es vorher statt erst beim Speichern.
 */
export type SecretStoreState =
  { available: true } | { available: false; os: "linux" };

export function secretStoreState(): SecretStoreState {
  let os: string | undefined;
  try {
    os = type();
  } catch {
    // Ohne Tauri (Vorschau, Tests) gilt der Windows-Pfad.
    return { available: true };
  }
  if (os === "linux") return { available: false, os };
  return { available: true };
}
