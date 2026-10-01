"""Gate: jede Kern-Funktion der Feature-Matrix ist gleichwertig/besser/vorhanden und belegt.

Aufruf: python koordination/granola-besprechungen/check_matrix.py
Exit 0 = alle Kern-Zeilen erfüllt; Exit 1 = offene Zeilen (werden aufgelistet); Exit 2 = Formatfehler.
"""

import pathlib
import sys

MATRIX = pathlib.Path(__file__).with_name("FEATURE-MATRIX.md")
ERFUELLT = {"gleichwertig", "besser", "vorhanden"}
KLASSEN = {"Kern", "Komfort", "Nein"}


def zeilen(text: str):
    for nr, zeile in enumerate(text.splitlines(), 1):
        if not zeile.startswith("| F"):
            continue
        zellen = [z.strip() for z in zeile.strip().strip("|").split("|")]
        if len(zellen) != 8:
            raise ValueError(f"Zeile {nr}: {len(zellen)} statt 8 Spalten")
        yield nr, zellen


def main() -> int:
    try:
        eintraege = list(zeilen(MATRIX.read_text(encoding="utf-8")))
    except ValueError as fehler:
        print(f"FORMATFEHLER: {fehler}")
        return 2
    if not eintraege:
        print("FORMATFEHLER: keine Feature-Zeilen gefunden")
        return 2

    offen = []
    kern = 0
    for nr, (fid, name, klasse, _ist, _ziel, _m, status, beleg) in eintraege:
        if klasse not in KLASSEN:
            print(f"FORMATFEHLER: Zeile {nr} ({fid}) unbekannte Klasse {klasse!r}")
            return 2
        if klasse != "Kern":
            continue
        kern += 1
        if status not in ERFUELLT or not beleg:
            offen.append(f"{fid} {name} — Status {status!r}, Beleg {'fehlt' if not beleg else 'ok'}")

    print(f"Kern-Funktionen: {kern}, erfüllt: {kern - len(offen)}, offen: {len(offen)}")
    for eintrag in offen:
        print(f"  offen: {eintrag}")
    return 1 if offen else 0


if __name__ == "__main__":
    sys.exit(main())
