import { type } from "@tauri-apps/plugin-os";

/**
 * Ob die App Zugangsdaten sicher speichern kann. Der Tresor
 * (`calendar::secret`) verschluesselt derzeit nur unter Windows (DPAPI); auf
 * macOS und Linux lehnt er jedes Speichern ab, ohne Klartext-Ersatz.
 * Die Oberflaeche sperrt deshalb dort alles, was ein Geheimnis braucht, und
 * sagt es vorher statt erst beim Speichern (Codex-Entscheidung 31.10.2026, Folge-Issue #134).
 */
export type SecretStoreState =
  { available: true } | { available: false; os: "macos" | "linux" };

export function secretStoreState(): SecretStoreState {
  let os: string | undefined;
  try {
    os = type();
  } catch {
    // Ohne Tauri (Vorschau, Tests) gilt der Windows-Pfad.
    return { available: true };
  }
  if (os === "macos" || os === "linux") return { available: false, os };
  return { available: true };
}
