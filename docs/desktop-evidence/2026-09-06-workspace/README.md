# Desktop-Oberfläche: Aufgaben zuerst

Stand: 06.09.2026, isolierter Branch `codex/apple-p0-p1`.

## Umgesetzt

- Startbereich mit Diktierkürzel und direkten Wegen zu Aufnahmen, Verlauf und Vorlesen.
- Kleine WAI-Bildmarke, unveränderte WAI-Farbtokens, normale Schriftgrößen.
- Beschriftete Seitenleiste ab 768 px; darunter Navigation unten mit vier Aufgaben
  und „Mehr“ für Modelle/Einstellungen. Escape schließt Mehr und stellt den Fokus wieder her.
- Letzte Ansicht bleibt gespeichert; Erststart ohne gespeicherte Ansicht öffnet Start.
- Aufnahmeoptionen in Einstellungen: Sprache/Modell unter Diktat, Aufbewahrung unter App/Speicher.
- Einstellungen mit Tastatursteuerung (Pfeile, Pos1/Ende), zusammengehörigen Tab-/Panel-Beschriftungen.
- Vorlesen: Text/Import/Wiedergabe zuerst. Stil und weitere Optionen aufklappbar.
  Seiten- und Dateilisten werden unter 1100 px zu Ablagen über/unter dem Editor.
- Größere Verlauf-Aktionen, kompakte Statusleiste und deutsche/englische neue Texte.

## Verifikation

- TypeScript: bestanden, keine Diagnosen.
- ESLint für gesamtes Frontend: bestanden, keine Diagnosen.
- Produktions-Frontend: gebaut; vorhandene Warnung zu JavaScript-Paketen über 500 kB.
- 6 Playwright-Tests: bestanden. Getestet wird die reale React-Oberfläche mit
  deterministischen nativen Testantworten: Navigation/Neustart, 390-px-Fenster,
  WAI hell/dunkel, Tastatur, Windows-Oberflächenpfad, 700-px-Vorlesen samt Optionen.
- Vier Screenshots visuell geprüft. Screenshots zeigen Testdaten, keine behaupteten
  echten Aufnahmen oder angeschlossenen Mikrofone.
- Keine Änderungen am nativen Desktop-Audiopfad. Kein frischer nativer Windows-Build
  oder Windows-Gerätetest; kein Mac-Installationspaket für diesen UI-Stand erzeugt.

Die ursprünglichen lokalen Änderungen im ursprünglichen Checkout bleiben erhalten.
Kein Push, Release oder Eingriff in main.

## Nächster Integrationsschritt

Mobile Aufzeichnungen, Export und Geräteverbindung sind ein separater, noch laufender
Ausbau. Die neue Oberfläche behauptet keine bereits verfügbare Synchronisierung oder
Fremd-App-Anrufaufnahme. Kopplung/Synchronisierung gehört in die vorhandenen App-
Einstellungen, die Wahl des Eingabegeräts zum bestehenden Mikrofonwähler.
