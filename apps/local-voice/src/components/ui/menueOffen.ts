/**
 * Offene Auswahlmenues (eigene Menues mit `role="menu"`/`listbox` und die
 * Auswahllisten von `Select`). Ein Tooltip des Ziels darf sie nicht
 * ueberdecken (Patrick: die Sprechblase lag ueber dem geoeffneten Menue).
 */
export const MENUE_SELECTOR =
  '[role="menu"], [role="listbox"], .app-select__menu';

interface MenueKnoten {
  contains(other: unknown): boolean;
}

interface MenueWurzel {
  querySelectorAll(selector: string): ArrayLike<MenueKnoten>;
}

/**
 * Ob in `root` ein Menue offen ist, das nicht das Ziel selbst enthaelt (ein
 * Tooltip an einem Menueeintrag bleibt erlaubt).
 */
export function istMenueOffen(
  root: MenueWurzel,
  target: unknown = null,
): boolean {
  const menues = root.querySelectorAll(MENUE_SELECTOR);
  for (let i = 0; i < menues.length; i += 1) {
    if (target == null || !menues[i].contains(target)) return true;
  }
  return false;
}
