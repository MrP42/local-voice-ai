#!/usr/bin/env python3
"""Erzeugt apps/apple/VoiceCore/Sources/VoiceCore/Ablage/AblageFts5Tabellen.swift aus
docs/superpowers/plans/ablage-index-vektoren.json (Schluessel `fts5`).

Die Tabellen beschreiben den FTS5-Tokenizer `unicode61 remove_diacritics 2` des SQLite, mit dem der Desktop seinen
Ablage-Index baut (Rust-Test `ablage::tests_index_paritaet`). Aufruf ohne Argumente aus dem Repo; mit `--pruefen` wird
nichts geschrieben, sondern nur verglichen (Exit 1 bei Abweichung).
"""
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[3]
JSON = ROOT / "docs/superpowers/plans/ablage-index-vektoren.json"
SWIFT = ROOT / "apps/apple/VoiceCore/Sources/VoiceCore/Ablage/AblageFts5Tabellen.swift"


def bereiche(text):
    """`a-b,c` (hex) -> flache Liste [a, b, c, c] (Paare von Anfang und Ende, beide eingeschlossen)."""
    aus = []
    for teil in filter(None, text.split(",")):
        if "-" in teil:
            a, b = teil.split("-")
        else:
            a = b = teil
        aus += [int(a, 16), int(b, 16)]
    return aus


def laeufe(text):
    """`start:anzahl:schritt:delta` (dezimal) -> flache Liste von Vierergruppen."""
    aus = []
    for teil in filter(None, text.split(",")):
        aus += [int(x) for x in teil.split(":")]
    return aus


def zeilen(zahlen, typ, je_zeile=12):
    zeilen = []
    for i in range(0, len(zahlen), je_zeile):
        zeilen.append("        " + ", ".join(str(z) for z in zahlen[i:i + je_zeile]) + ",")
    return "\n".join(zeilen)


def erzeuge():
    fts5 = json.loads(JSON.read_text(encoding="utf-8"))["fts5"]
    wort = bereiche(fts5["token_zeichen"])
    folge = bereiche(fts5["folge_zeichen"])
    falten = laeufe(fts5["falten"])
    entfernen = bereiche(fts5["entfernen"])
    return f"""// GENERIERT von apps/apple/scripts/gen_ablage_fts5_tabellen.py aus docs/superpowers/plans/ablage-index-vektoren.json
// (Schluessel `fts5`, erzeugt vom Rust-Test `ablage::tests_index_paritaet` aus dem SQLite des Desktop-Index). Nicht von Hand aendern;
// `AblageIndexParitaetTests` vergleicht diese Reihen mit der Datei.
//
// Verhalten des FTS5-Tokenizers `unicode61 remove_diacritics 2` je Code-Punkt:
// - `wortzeichen`: Paare (von, bis) einschliesslich: Zeichen, die ein Wort beginnen und fortsetzen.
// - `folgezeichen`: Paare: Zeichen, die ein Wort nur fortsetzen (Diakritika).
// - `falten`: Vierergruppen (start, anzahl, schritt, delta): Code-Punkt start + k*schritt wird zu start + k*schritt + delta.
// - `entfernen`: Paare: Zeichen, die im Wort verschwinden.
// Alle anderen Code-Punkte trennen Woerter; nicht aufgefuehrte Zeichen falten auf sich selbst.

enum AblageFts5Tabellen {{
    static let wortzeichen: [UInt32] = [
{zeilen(wort, "UInt32")}
    ]

    static let folgezeichen: [UInt32] = [
{zeilen(folge, "UInt32")}
    ]

    static let falten: [Int32] = [
{zeilen(falten, "Int32", 8)}
    ]

    static let entfernen: [UInt32] = [
{zeilen(entfernen, "UInt32")}
    ]
}}
"""


def main():
    neu = erzeuge()
    if "--pruefen" in sys.argv:
        if not SWIFT.exists() or SWIFT.read_text(encoding="utf-8") != neu:
            print("AblageFts5Tabellen.swift weicht von der JSON-Datei ab", file=sys.stderr)
            sys.exit(1)
        print("AblageFts5Tabellen.swift passt zur JSON-Datei")
        return
    SWIFT.write_text(neu, encoding="utf-8")
    print(f"geschrieben: {SWIFT} ({len(neu)} Bytes)")


if __name__ == "__main__":
    main()
