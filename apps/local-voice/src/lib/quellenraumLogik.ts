import type { QuellenraumSicht, QuellenZeile } from "../bindings";

export const artSchluessel = (z: QuellenZeile): string => z.art;

export const zustandSchluessel = (
  z: QuellenZeile,
): "bereit" | "kein_text" | "nicht_verfuegbar" => z.state;

export const auswahlText = (
  s: QuellenraumSicht,
): { count: number; total: number } => ({
  count: s.selectedCount,
  total: s.total,
});

export const versionLabel = (
  z: QuellenZeile,
): { nr: number; anzahl: number } | null =>
  z.version === null ? null : { nr: z.version, anzahl: z.versionCount };

export const nichtWaehlbar = (z: QuellenZeile): boolean =>
  z.state === "nicht_verfuegbar";

export function sortiert(zeilen: readonly QuellenZeile[]): QuellenZeile[] {
  return zeilen
    .map((zeile, index) => ({ zeile, index }))
    .sort((a, b) => {
      const auswahl = Number(b.zeile.selected) - Number(a.zeile.selected);
      if (auswahl !== 0) return auswahl;
      const aOhne = nichtWaehlbar(a.zeile) && a.zeile.title === null;
      const bOhne = nichtWaehlbar(b.zeile) && b.zeile.title === null;
      if (aOhne !== bOhne) return Number(aOhne) - Number(bOhne);
      return (
        (a.zeile.title ?? "").localeCompare(b.zeile.title ?? "", "de") ||
        a.index - b.index
      );
    })
    .map(({ zeile }) => zeile);
}
