# Aufnahmen

Besprechungen aufnehmen oder Dateien importieren, mitschreiben, transkribieren und zu Notizen und einem Protokoll verdichten. Alles bleibt auf diesem Rechner.

## So ist die Seite aufgebaut

- **Links: Projekte.** „Alle Aufnahmen“, „Ohne Projekt“ und deine eigenen Projekte, etwa „Privat“ oder „Kunde Stadtwerke“. Das Symbol „Neues Projekt“ legt eins an; Doppelklick oder F2 benennt um, Alt+Pfeil hoch/runter sortiert, das Kontextmenü (rechte Maustaste) bietet dasselbe. Die Besprechungen des gewählten Projekts stehen darunter; Suche und Filter wirken nur dort.
- **Mitte: Arbeitsfläche.** Der Titel (Klick benennt um), die Reiter **Notizen**, **KI-Notizen** und **Protokoll** und darunter dein Text.
- **Rechts: Bedienung.** Oben Aufnahme starten und Datei importieren, darunter die Symbolzeile (Exportieren, Follow-up-Mail, Kopieren, Personen) und das Menü ☰. Weiter unten stehen **Transkript** und **Fragen**.
- Die Griffe zwischen den Spalten ziehst du mit der Maus oder verstellst sie mit den Pfeiltasten; Doppelklick stellt die Standardbreite her, die Pfeil-Symbole am Rand klappen eine Spalte ein. Breiten, Auswahl und Reiter bleiben beim nächsten Start erhalten. Jede Spalte scrollt für sich, die Seite nie.

## Projekte und Besprechungen ordnen

- Eine Besprechung ziehst du auf ein Projekt, um sie **zu verschieben**. Mit **Strg+Ziehen** legst du sie **zusätzlich** dorthin; sie liegt dann in mehreren Projekten.
- Ohne Maus: Menü ☰ oder der Projekt-Chip im Kopf, „In Projekt verschieben …“.
- Ein Projekt zu löschen löscht keine Besprechung; sie stehen danach unter „Ohne Projekt“.

## Aufnehmen

- **Mikrofon** nimmt dich auf, **System-Audio** zusätzlich das, was der Rechner wiedergibt, etwa die Gegenseite eines Videocalls. System-Audio gibt es unter Windows, auf dem Mac läuft die Aufnahme nur über das Mikrofon.
- Vor der ersten Aufnahme fragt die App nach der Einwilligung der Teilnehmer. Während der Aufnahme ist das Diktat gesperrt.
- Schreibe mit: Stichpunkte im Reiter **Notizen** genügen, jeder Punkt merkt sich die Aufnahmezeit. Das Live-Transkript läuft rechts mit.
- Stürzt die App ab, wird die Aufnahme beim nächsten Start repariert und steht wieder in der Liste.

## Importieren

Audio- und Videodateien sowie Untertitel (VTT, SRT) importierst du mit dem Symbol **Datei importieren** in der Bedienung oder indem du sie auf die Arbeitsfläche ziehst; mehrere Dateien auf einmal gehen auch. Sie landen im gewählten Projekt und werden wie eine Aufnahme transkribiert.

### Warteschlange

- Weitere Dateien kannst du **jederzeit** hinzufügen, auch während eine andere noch transkribiert wird. Jede bekommt sofort ihre Besprechung mit dem Status **Wartet** und ihrem **Platz** („Platz 2 von 3“, in der Liste und im Kopf). Die Dateien laufen in der Reihenfolge, in der du sie hinzugefügt hast.
- Eine wartende Datei ziehst du im Kontextmenü (rechte Maustaste) oder in der Bedienspalte **nach vorn** oder nimmst sie **aus der Warteschlange**; sie gilt dann als abgebrochen, und **Wieder einreihen** stellt sie hinten an. Eine laufende Datei stoppst du wie bisher.
- Die Warteschlange bleibt über einen Neustart erhalten. Wurde eine Datei gelöscht oder verschoben, bevor sie an der Reihe war, meldet die Besprechung das, und es geht mit der nächsten weiter.
- **Eine Aufnahme hat immer Vorrang:** Solange sie läuft, beginnt keine neue Datei, und laufende Importe halten am nächsten Block an. Danach geht es von selbst weiter.
- Unter Einstellungen, Diktat, Besprechungen steht **Gleichzeitige Transkriptionen** (1, 2 oder 3; Standard 1). Jede weitere Transkription lädt das Modell noch einmal und braucht deshalb Platz: Reichen Arbeitsspeicher (und bei Grafikkarten-Modellen Grafikspeicher) nicht, wartet die nächste Datei mit dem Hinweis „Wartet auf Arbeitsspeicher“. Der Rechner wird dabei nie ausgelastet bis zum Stillstand; bei Zweifel bleibt es bei einer.

## Notizen, KI-Notizen und Protokoll

- **Notizen** sind dein Text und bleiben es. Die **KI-Notizen** entstehen aus deinen Stichpunkten und dem Transkript; jede Aussage hat einen Beleg, ein Klick darauf springt ins Transkript.
- Das **Protokoll** (Zusammenfassung, Entscheidungen, Aufgaben) schreibt das Sprachmodell aus der Fußleiste. Dafür muss ein Sprachmodell geladen oder ein Anbieter verbunden sein.
- Die **Vorlage** legt die Abschnitte fest. Bei „Automatisch“ wählt die App eine passende; im Menü ☰ wechselst du sie mit „Vorlage wechseln …“ oder erzeugst Notizen und Protokoll neu.
- Die Transkription läuft mit dem Modell aus der Fußleiste, oder mit einem eigenen unter Einstellungen, Diktat, Besprechungen.

## Fortschritt, Pause und Stopp

Während eine Besprechung verarbeitet wird, zeigt ein Fortschrittsbalken Phase, Prozent und Restdauer. **Pausieren** hält die Verarbeitung an und gibt den Rechner frei, **Fortsetzen** macht weiter. **Stoppen** bricht ab; das bisherige Transkript bleibt, du kannst später fortsetzen oder neu transkribieren.

## Menü ☰ und Details

Seltene Aktionen stehen im Menü ☰: Neu transkribieren, KI-Notizen und Protokoll neu erzeugen, Vorlage wechseln, in Projekt verschieben, Umbenennen, Details und Löschen. **Details** zeigt Status, Quelle, Dauer, Einwilligung, Modell und Aufbewahrung auf einen Blick.

Mit **Bearbeiten** im Details-Dialog änderst du **Titel**, **Beschreibung** (mehrzeilig), **Datum und Uhrzeit**, die **Teilnehmenden** (aus den vorhandenen Personen) und die **Projekte**. Gespeichert wird alles oder nichts; Dateiname und Quelle bleiben, wie sie sind. Die Beschreibung ist durchsuchbar und steht dem Chat, den KI-Notizen, dem Protokoll und dem lokalen MCP-Server als Hintergrund zur Verfügung.

## Kleines Fenster

Wird das Fenster schmal, etwa neben einem Videocall, klappen die Projekte in eine Schublade („Projekte öffnen“), und Arbeitsfläche und Transkript teilen sich die Höhe. Den Trenner dazwischen stellst du mit der Maus oder mit Pfeil hoch/runter ein. Aufnehmen, mitschreiben und das Live-Transkript lesen geht auch so, ohne Seitenwechsel.

## Aufbewahrung

Standardmäßig wird das Audio gelöscht, sobald das Protokoll steht. Transkript und Protokoll bleiben. Einstellbar unter Einstellungen, Allgemein.
