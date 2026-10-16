/**
 * Springt in einen Reiter der Einstellungen, optional zu einer Stelle darin.
 *
 * Der Reiter steht in localStorage (so liest ihn eine noch nicht geoeffnete
 * Einstellungsseite beim Start), das Ereignis `lv-open-settings-tab` setzt ihn
 * in einer schon geoeffneten Seite. Der Anker (`data-settings-anchor`) bleibt
 * vorgemerkt, bis die Seite ihn gefunden und angescrollt hat -- die Inhalte
 * eines Reiters erscheinen erst nach dem Wechsel.
 */
export type SettingsTabId = "input" | "output" | "models" | "app" | "about";

export const OPEN_SETTINGS_TAB = "lv-open-settings-tab";

let pendingAnchor: string | null = null;

export function openSettingsTab(tab: SettingsTabId, anchor?: string): void {
  pendingAnchor = anchor ?? null;
  try {
    window.localStorage.setItem("lva.ui.settings.tab", tab);
  } catch {
    /* ohne Speicher wirkt nur das Ereignis */
  }
  window.dispatchEvent(
    new CustomEvent("lv-navigate", { detail: { section: "settings" } }),
  );
  window.dispatchEvent(new CustomEvent(OPEN_SETTINGS_TAB, { detail: { tab } }));
}

/** Scrollt zum vorgemerkten Anker, sobald er im Dokument steht (bis ~2 s). */
export function scrollToPendingAnchor(): void {
  const anchor = pendingAnchor;
  if (!anchor) return;
  let tries = 0;
  const attempt = () => {
    const el = document.querySelector(`[data-settings-anchor="${anchor}"]`);
    if (el) {
      pendingAnchor = null;
      el.scrollIntoView({ block: "start", behavior: "smooth" });
      return;
    }
    if (pendingAnchor === anchor && ++tries < 20) window.setTimeout(attempt, 100);
  };
  window.setTimeout(attempt, 0);
}
