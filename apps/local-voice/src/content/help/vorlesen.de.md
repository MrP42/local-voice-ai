# Vorlesen

Text in die Mitte, Stimme wählen, Vorlesen drücken. Alles läuft auf diesem Rechner, nichts verlässt ihn.

## Was diese Seite kann

- **Text** tippen, einfügen oder diktieren (Mikrofon-Symbol).
- **Symbolzeile** unter der Stimmenwahl: lauter gleich große Symbole ohne Beschriftung. Ein Tooltip nennt Namen und Wirkung, per Maus nach kurzem Verweilen, per Tastatur beim Fokus. Seltenes liegt hinter dem Menü (☰) am rechten Rand.
- **Hinzufügen (+)**: ein Dokument (TXT, MD, PDF, DOCX), eine Web-Adresse oder eine Datei ins Projekt holen.
- **Übersetzen** (Sprachsymbol, im Reiter Übersetzung, daneben die Zielsprache) und **Zusammenfassen** (Dokumentsymbol, im Reiter Zusammenfassung; Umfang, Detailgrad und Zielgruppe stehen hinter dem Reglersymbol daneben) legen das Ergebnis in einen eigenen Reiter. Das Original bleibt unverändert.
- **Vorlesen** liest Satz für Satz. Die Pfeile springen zum vorigen oder nächsten Satz, Pause hält an.
- **Als Audio speichern** (Pfeil nach unten) schreibt die Aufnahme in den Projektordner der Seite. Sie erscheint rechts unter Dateien.
- **Änderungen vorab erzeugen** (Blitz) legt geänderte Sätze im Voraus im Cache ab, ohne abzuspielen.
- **Menü (☰)**: Skript-Werkstatt, Text aufbereiten, Skript prüfen und Auto-Tagging. Gefundene Skript-Fehler zeigt eine rote Zahl am Menüsymbol.

## Seiten (links)

Jede Seite ist ein Arbeitsblatt mit eigenem Text und eigenem Ordner. Die Liste zeigt den Anfang des Texts und wann er zuletzt geändert wurde. Doppelklick benennt um.

<!--if:fish-->
## Sprecher und Betonung im Text

- **Sprecherwechsel**: eine Zeile mit dem Namen einer Stimme und Doppelpunkt beginnen, zum Beispiel `Olga:`. Alles bis zum nächsten Wechsel spricht diese Stimme.
- **Stil**: `<Olga:flüsternd>` wählt einen gespeicherten Stil dieser Stimme.
- **Tags** stehen in eckigen Klammern genau dort, wo sie wirken sollen: `[whisper] Komm näher.` oder `Er öffnete die Tür. [short pause] Nichts.`
- **Auto-Tagging** (Menü ☰) schlägt Tags per Sprachmodell vor. Es fügt nur ein, es löscht nichts. Vorschläge lassen sich einzeln übernehmen oder mit Rückgängig verwerfen.
- Die Palette unter dem Text listet alle Tags nach Gruppen. Klick fügt an der Cursorposition ein.

<!--/if:fish-->

## Stimmen

- Ausgewählt wird in der Leiste über dem Player. Piper-Stimmen stehen dort mit Sprache und Qualität, etwa „Thorsten · Deutsch · HQ · Piper". Jeder Reiter merkt sich seine Stimme.
<!--if:fish-->
- Anhören, klonen, importieren und löschen: **Einstellungen → Ausgabe**, oder direkt über „Stimmen verwalten …" am Ende der Stimmenliste.
<!--/if:fish-->
<!--if:fish-->
- **Klonen** braucht eine Referenz von 10 bis 30 Sekunden. Das Transkript entsteht automatisch und lässt sich korrigieren.
- Der **Seed** bestimmt, wie die Standardstimme klingt. Ein gefundener Seed lässt sich als benannte Stimme sichern.
- Sprecherwechsel im Text funktionieren mit Fish-Speech-Stimmen. Piper liest alles in der gewählten Stimme.
<!--/if:fish-->
<!--if:nofish-->
- Piper liest den ganzen Text in der gewählten Stimme. Sprecherwechsel, Klonen und Stile gehören zu **Fish Speech**, einer optionalen Zusatz-Engine für die Grafikkarte. Sie ist auf diesem Rechner nicht eingerichtet; den Ordner trägst du unter **Einstellungen → Ausgabe** ein.
<!--/if:nofish-->

<!--if:fish-->
## Zwei Engines

| | Fish Speech | Piper |
|---|---|---|
| Läuft auf | GPU (NVIDIA, ab 6 GB VRAM) | CPU |
| Stimmen | geklont, Seed, Stile, Tags | feste Katalogstimmen |
| Start | Server, 20 bis 90 s | sofort |
| Qualität | natürlich, betont | klar, gleichmäßig |

Die Engine steht unter **Einstellungen → Ausgabe**. Piper-Stimmen lädt die Modelle-Seite unter Vorlesestimmen.
<!--/if:fish-->
<!--if:nofish-->
## Sprachausgabe einrichten

Piper läuft auf der CPU, startet sofort und braucht nur eine kleine Stimme. Laden: **Modelle → Vorlesestimmen**. Ist das Piper-Programm unvollständig, steht die Stimme dort als „nicht nutzbar“ und lässt sich mit „Programm installieren“ reparieren.
<!--/if:nofish-->

## Sprachmodell und Server in der Fußleiste

- **Sprachmodell** (Fußleiste, Aufklappmenü): das Modell für Übersetzen, Zusammenfassen und Auto-Tagging. Die Ampel pulsiert gelb, solange es arbeitet; im Menü „Vorwärmen“ (lädt es für zehn Minuten) und „Entladen“ (gibt den Speicher frei).
<!--if:fish-->
- **Server** (Fußleiste, links neben dem Schild): der Fish-Speech-Server. Grau aus, gelb startet, grün läuft, orange Fehler. Ein Klick fragt nach: starten, neu starten oder beenden.
<!--/if:fish-->

## Wenn etwas hakt

<!--if:fish-->
- **Start dauert lange**: andere GPU-Programme schließen, der Server braucht freien Videospeicher.
<!--/if:fish-->
- **Text wird gekürzt**: die Grenze steht unter Einstellungen → Ausgabe, maximale Zeichen pro Auftrag.
<!--if:fish-->
- **Weiße Seite oder Fehlermeldung im Kopf**: Server stoppen und neu starten. Bleibt es, den Fish-Speech-Ordner in den Einstellungen prüfen.
<!--/if:fish-->
<!--if:nofish-->
- **Stimme „nicht eingerichtet“**: unter **Modelle → Vorlesestimmen** laden oder reparieren; danach erscheint sie sofort in der Auswahl.
<!--/if:nofish-->

