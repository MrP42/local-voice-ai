/**
 * Tastaturregeln des Buch-Gesprächs (lva#309, Plan §9): wann eine Taste dem
 * Gespräch gehört und wann sie jemand anderem.
 */

/** Wurzel der Gesprächsspalte; ein Dialog, der sie enthält, ist „eigen“ (das Blatt „Gespräch“). */
export const BOOKTALK_ROOT_ATTR = "data-booktalk-root";

/** Texteingabe: dort bedeuten Leertaste und G Text. Knöpfe und Links zählen nicht. */
export const isTextEntry = (target: EventTarget | null): boolean =>
  target instanceof Element &&
  target.closest(
    'input, textarea, select, [contenteditable=""], [contenteditable="true"]',
  ) !== null;

/**
 * Liegt das Ziel in einem fremden Dialog (Bestätigung, Einstellungen, anderes
 * Blatt)? Der eigene Blatt-Dialog mit der Gesprächsspalte zählt nicht, dort
 * gelten Esc, Leertaste und G wie in der Spalte.
 */
export const inForeignDialog = (target: EventTarget | null): boolean => {
  if (!(target instanceof Element)) return false;
  const dialog = target.closest('[role="dialog"], [role="alertdialog"]');
  if (!dialog) return false;
  return dialog.querySelector(`[${BOOKTALK_ROOT_ATTR}]`) === null;
};
