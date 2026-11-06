/** Klassennamen verbinden; leere Werte entfallen (Ersatz für `cx` aus @wai/ui). */
export function cx(...teile: (string | false | null | undefined)[]): string {
  return teile.filter(Boolean).join(" ");
}
