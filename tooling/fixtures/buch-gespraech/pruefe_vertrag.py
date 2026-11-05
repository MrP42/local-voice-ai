"""Prüft die Beispiele gegen die Einstiegsschemas des Buch-Gesprächs (lva#307).

Aufruf: python3 tooling/fixtures/buch-gespraech/pruefe_vertrag.py
Benötigt jsonschema >= 4.18 (MIT). Exit 0 = alle gültigen Beispiele gültig und
alle ungültigen abgelehnt.
"""
import json
import sys
from pathlib import Path

from jsonschema import Draft202012Validator
from referencing import Registry, Resource

HIER = Path(__file__).resolve().parent
ENTRIES = ["request", "event", "answer", "stored-turn", "command"]


def lade(name):
    return json.loads((HIER / name).read_text(encoding="utf-8"))


def main():
    schemas = {f"{e}.schema.json": lade(f"{e}.schema.json") for e in ENTRIES}
    schemas["vertrag.schema.json"] = lade("vertrag.schema.json")
    registry = Registry().with_resources(
        (s["$id"], Resource.from_contents(s)) for s in schemas.values()
    )
    validators = {
        e: Draft202012Validator(schemas[f"{e}.schema.json"], registry=registry) for e in ENTRIES
    }
    validators["root"] = Draft202012Validator(schemas["vertrag.schema.json"], registry=registry)
    fehler = 0
    for datei, soll in (("beispiele/gueltig.json", True), ("beispiele/ungueltig.json", False)):
        for art, faelle in lade(datei).items():
            for i, fall in enumerate(faelle):
                ist = validators[art].is_valid(fall)
                if ist != soll:
                    fehler += 1
                    print(f"FEHLER {datei} {art}[{i}]: gueltig={ist}, erwartet={soll}")
    print("Vertrag: GRUEN" if fehler == 0 else f"Vertrag: ROT ({fehler})")
    return 1 if fehler else 0


if __name__ == "__main__":
    sys.exit(main())
