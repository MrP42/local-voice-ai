# Vorlesen

Text in die Mitte, Stimme wählen, Vorlesen drücken. Alles läuft auf diesem Rechner, nichts verlässt ihn.

## Was diese Seite kann

- **Text** tippen, einfügen oder diktieren (Mikrofon-Knopf).
- **Hinzufügen (+)**: ein Dokument (TXT, MD, PDF, DOCX), eine Web-Adresse oder eine Datei ins Projekt holen.
- **Übersetzen** und **Zusammenfassen** legen das Ergebnis in einen eigenen Reiter. Das Original bleibt unverändert.
- **Vorlesen** liest Satz für Satz. Die Pfeile springen zum vorigen oder nächsten Satz, Pause hält an.
- **Audio speichern** schreibt die Aufnahme in den Projektordner der Seite. Sie erscheint rechts unter Dateien.

## Seiten (links)

Jede Seite ist ein Arbeitsblatt mit eigenem Text und eigenem Ordner. Die Liste zeigt den Anfang des Texts und wann er zuletzt geändert wurde. Doppelklick benennt um.

## Sprecher und Betonung im Text

- **Sprecherwechsel**: eine Zeile mit dem Namen einer Stimme und Doppelpunkt beginnen, zum Beispiel `Olga:`. Alles bis zum nächsten Wechsel spricht diese Stimme.
- **Stil**: `<Olga:flüsternd>` wählt einen gespeicherten Stil dieser Stimme.
- **Tags** stehen in eckigen Klammern genau dort, wo sie wirken sollen: `[whisper] Komm näher.` oder `Er öffnete die Tür. [short pause] Nichts.`
- **Auto-Tagging** schlägt Tags per Sprachmodell vor. Es fügt nur ein, es löscht nichts. Vorschläge lassen sich einzeln übernehmen oder mit Rückgängig verwerfen.
- Die Palette unter dem Text listet alle Tags nach Gruppen. Klick fügt an der Cursorposition ein.

## Stimmen

- Ausgewählt wird in der Leiste über dem Player.
- Anhören, klonen, importieren und löschen: **Modelle → Stimmen anhören & verwalten**.
- **Klonen** braucht eine Referenz von 10 bis 30 Sekunden. Das Transkript entsteht automatisch und lässt sich korrigieren.
- Der **Seed** bestimmt, wie die Standardstimme klingt. Ein gefundener Seed lässt sich als benannte Stimme sichern.
- Sprecherwechsel im Text funktionieren mit Fish-Speech-Stimmen. Piper liest alles in der gewählten Stimme.

## Zwei Engines

| | Fish Speech | Piper |
|---|---|---|
| Läuft auf | GPU (NVIDIA, ab 6 GB VRAM) | CPU |
| Stimmen | geklont, Seed, Stile, Tags | feste Katalogstimmen |
| Start | Server, 20 bis 90 s | sofort |
| Qualität | natürlich, betont | klar, gleichmäßig |

Die Engine steht unter **Einstellungen → Vorlesen**. Piper-Stimmen lädt die Modelle-Seite unter Vorlesestimmen.

## Symbole im Seitenkopf

- **Gehirn**: das Sprachmodell für Übersetzen, Zusammenfassen und Auto-Tagging. Klick lädt es vor oder entlädt es.
- **Server**: der Fish-Speech-Server. Grau aus, gelb startet, grün läuft, orange Fehler. Klick tut, was in diesem Zustand ansteht.

## Wenn etwas hakt

- **Start dauert lange**: andere GPU-Programme schließen, der Server braucht freien Videospeicher.
- **Text wird gekürzt**: die Grenze steht unter Einstellungen → Vorlesen, maximale Zeichen pro Auftrag.
- **Weiße Seite oder Fehlermeldung im Kopf**: Server stoppen und neu starten. Bleibt es, den Fish-Speech-Ordner in den Einstellungen prüfen.
