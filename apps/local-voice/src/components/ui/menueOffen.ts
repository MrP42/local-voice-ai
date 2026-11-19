/**
 * Offene Auswahlmenues (eigene Menues mit `role="menu"`/`listbox` und die
 * Auswahllisten von `Select`). Ein Tooltip des Ziels darf sie nicht
 * ueberdecken (Patrick: die Sprechblase lag ueber dem geoeffneten Menue).
 */
export const MENUE_SELECTOR =
  '[role="menu"], [role="listbox"], .app-select__menu';

interface MenueKnoten {
  contains(other: unknown): boolean;
  hidden?: boolean;
  getAttribute?(name: string): string | null;
}

interface MenueWurzel {
  querySelectorAll(selector: string): ArrayLike<MenueKnoten>;
}

/**
 * Ob das Menue zu sehen ist: nicht `hidden`, nicht `aria-hidden="true"` und
 * nicht `visibility:hidden` (z. B. vor der Platzierung gemountet).
 */
export function istMenueSichtbar(menue: MenueKnoten): boolean {
  if (menue.hidden === true) return false;
  if (menue.getAttribute?.("aria-hidden") === "true") return false;
  if (
    typeof window !== "undefined" &&
    typeof window.getComputedStyle === "function" &&
    typeof Element !== "undefined" &&
    menue instanceof Element &&
    window.getComputedStyle(menue).visibility === "hidden"
  ) {
    return false;
  }
  return true;
}

/**
 * Ob in `root` ein sichtbares Menue offen ist, das nicht das Ziel selbst
 * enthaelt (ein Tooltip an einem Menueeintrag bleibt erlaubt).
 */
export function istMenueOffen(
  root: MenueWurzel,
  target: unknown = null,
  sichtbar: (menue: MenueKnoten) => boolean = istMenueSichtbar,
): boolean {
  const menues = root.querySelectorAll(MENUE_SELECTOR);
  for (let i = 0; i < menues.length; i += 1) {
    if (!sichtbar(menues[i])) continue;
    if (target == null || !menues[i].contains(target)) return true;
  }
  return false;
}
