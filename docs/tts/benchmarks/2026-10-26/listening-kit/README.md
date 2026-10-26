# Blindhörtest: Anleitung

Sie hören 40 kurze Sprachproben. Die Proben stammen von verschiedenen Sprachausgaben, **welche das ist, wird
Ihnen nicht gesagt** (und steht nicht in diesem Ordner). Die Reihenfolge ist gemischt, die Dateinamen sind
Zufalls-IDs.

## Ablauf

1. Öffnen Sie `bewertungsbogen.csv` (Excel/LibreOffice; Trennzeichen Semikolon, UTF-8).
2. Hören Sie jede Datei in `proben/` mindestens einmal ganz an, mit demselben Kopfhörer und gleicher Lautstärke.
   Die Proben sind auf ähnliche Lautstärke angeglichen.
3. Tragen Sie je Zeile in `natuerlichkeit`, `aussprache`, `prosodie` und `artefakte` eine ganze Zahl von 1 bis 5 ein
   (5 ist jeweils das beste Ergebnis). Leer lassen, wenn Sie eine Probe nicht beurteilen können.
4. Optional: `freitext` für Auffälligkeiten, `hoerer` für Ihr Kürzel (damit mehrere Bögen zusammengeführt werden können).
5. Speichern Sie den Bogen als CSV (nicht als .xlsx) und geben Sie ihn zurück.

## Skala

| Merkmal | 1 | 5 |
|---|---|---|
| Natürlichkeit | klar künstlich/roboterhaft | von einer Menschenstimme kaum zu unterscheiden |
| Aussprache | viele falsch gesprochene Wörter/Zahlen/Namen | alles korrekt gesprochen |
| Prosodie (Betonung, Satzmelodie, Pausen) | monoton oder unpassend | passend zum Inhalt (Frage, Ausruf, Aufzählung) |
| Artefakte (Knacken, Rauschen, Abbrüche, Verzerrung) | stark störend | keine hörbaren Störungen |

## Hinweise für die Durchführung

- Nicht in Gruppen bewerten und nicht über die Proben sprechen, bevor alle Bögen abgegeben sind.
- Der Schlüssel liegt getrennt beim Auswertenden: `python scripts/bench/tts/tts_bench.py listening-eval --key <key.json> --sheets <bogen.csv> ...`.
- Seed dieses Pakets: siehe Schlüssel (steht ebenfalls nur im Schlüssel-Ordner).
