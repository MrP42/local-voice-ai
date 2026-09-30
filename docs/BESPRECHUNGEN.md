# Besprechungen aufnehmen, ordnen und wiederfinden

Kurzanleitung für die Seite **Aufnahmen** in der Seitenleiste (Stand 30.09.2026).
Alles läuft auf diesem Rechner, ohne Konto, ohne Abo und ohne Bot im Videocall. Was
noch nicht geht, steht in [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md#besprechungen-stand-2026-09-29).

## Die Seite im Überblick

Drei Spalten, jede scrollt für sich; Breiten, Auswahl und Reiter bleiben über einen Neustart erhalten.

- **Projekte** (links): „Alle Aufnahmen“, „Ohne Projekt“ und Ihre Projekte, darunter ihre Besprechungen, oben Suche und Filter; **Als Nächstes** zeigt den nächsten Termin.
- **Arbeitsfläche** (Mitte): Titel und die Reiter **Notizen**, **KI-Notizen**, **Protokoll**.
- **Bedienung** (rechts): Aufnahme, Import, Symbolzeile und Menü **☰** (Neu transkribieren, Vorlage, Verschieben, Details, Löschen); darunter **Transkript** und **Fragen**.
- **Schmales Fenster** (neben dem Videocall): Projekte in einer Schublade, Arbeitsfläche und Transkript teilen sich die Höhe.

## Aufnehmen und mitschreiben

1. In der Bedienung **Aufnahme starten** wählen; der Startdialog fragt Titel, Projekt und Vorlage
   (oder unter **Als Nächstes** einen Kalendertermin aufnehmen, siehe unten).
2. Beim ersten Start bestätigen Sie, dass **alle Beteiligten zugestimmt haben** (§ 201 StGB). Die App
   hält einen Hinweistext zum Kopieren in den Meeting-Chat bereit.
3. **System-Audio (Gegenseite) mitschneiden** ist vorgewählt: Mikrofon = „Ich“, Systemton = „Gegenseite“.
   Ohne Kopfhörer rechnet die **Echo-Unterdrückung** die Lautsprecher aus Ihrer Spur, damit nichts doppelt steht.
4. Im **Notizblock** genügen Stichpunkte. Jeder Punkt merkt sich die Aufnahmezeit; Aufgaben lassen sich abhaken.
   Der Block wird laufend gesichert, auch bei einem Absturz.
5. Das **Live-Transkript** erscheint mit wenigen Sekunden Verzögerung. Zeigt die Warnleiste „kein Signal“
   oder „übersteuert“, stimmt am Mikrofon oder am Systemton etwas nicht.
6. **Beenden**: Ein **Enddurchlauf** mit einem genaueren Modell ersetzt das Live-Transkript
   (Einstellungen → Diktat → Besprechungen → Enddurchlauf; „Aus“ behält das Live-Transkript).

Während einer Aufnahme ist das Diktat gesperrt. Dateien (Audio, Video, VTT, SRT) importieren Sie mit dem Symbol
**Datei importieren** oder indem Sie sie auf die Arbeitsfläche ziehen; sie landen im gewählten Projekt.
Bei laufender Verarbeitung: **Pausieren** gibt den Rechner frei, **Stoppen** behält das bisherige Transkript.

## KI-Notizen und Vorlagen

Nach dem Stopp macht die KI aus Ihren Stichpunkten und dem Transkript geordnete Notizen
(Reiter **KI-Notizen**). Automatisch nur, wenn ein Sprachmodell eingerichtet ist (Einstellungen → Sprachmodelle);
sonst per **KI-Notizen erzeugen**.

- **Ihr Text bleibt Ihr Text**: Er wird nie verändert und ist vom KI-Text (grau) unterscheidbar.
- **Jede KI-Aussage hat einen Beleg.** Ein Klick auf die Quelle springt ins Transkript und ins Audio.
  Einträge „ohne Beleg“ sind markiert und wollen selbst geprüft werden.
- Klicken zum Bearbeiten; danach zählt der Eintrag als Ihr Text. Mit einer Anweisung (z. B. „kürzer“)
  lassen sich die Notizen umschreiben. Aufgaben stehen als Checkliste da.
- **Vorlagen** legen die Abschnitte fest. Mitgeliefert sind acht deutsche (Allgemein, Kundengespräch, Jour fixe,
  Projekt-Kickoff, Interview, Workshop, Lenkungskreis). Gewechselt wird im Menü ☰ (**Vorlage wechseln …**); „Automatisch“ wählt nach Inhalt.
  **Vorlagen verwalten …**: eigene anlegen, duplizieren, als Datei (`.lvtemplate.json`) austauschen.
  Die Standardvorlage wählen Sie in den Einstellungen.

## Sprecher

Nach dem Stopp, beim Import und bei der Neu-Transkription trennt ein lokales Modell die Sprecher je Kanal
(bis zu vier) und beschriftet sie „Gegenseite 1“, „Person 2“ usw. Im Transkript klicken Sie auf den
Sprecher, um ihn zu **benennen**, mit einem anderen **zusammenzuführen** oder **nur ein Segment** umzuhängen. Namen
überstehen eine Neu-Transkription. Sitzen mehrere Personen am selben Mikrofon, aktivieren Sie
**Mehrere Personen am Mikrofon**. Ausschalten: Einstellungen → Diktat → Besprechungen → Sprecher automatisch trennen.
Das Modell (NVIDIA Sortformer) wird bei Bedarf geladen; seine Lizenz steht unter Info → Danksagungen.

## Chat und Suche

- **Suche** (oben in der Projekte-Spalte): Stichworte im gewählten Projekt (oder über „Alle Aufnahmen“), Filter nach Zeitraum, Quelle und „Mit Notizen“.
  Mit dem Suchmodell **BGE-M3** (einmalig 635 MB, Einstellungen → Besprechungen → Semantische Suche) findet sie
  auch nach Bedeutung.
- **Projekte**: Eine Besprechung kann in mehreren Projekten liegen. Ziehen auf ein Projekt verschiebt, **Strg+Ziehen** legt dazu;
  ohne Maus über ☰ → **In Projekt verschieben …**. Löschen eines Projekts löscht keine Besprechung.
- **Fragen** (Reiter rechts, Strg+J) öffnet den Chat zu einer Besprechung, auch während der Aufnahme, oder über alle
  Besprechungen eines Projekts (Symbol **Alle Besprechungen fragen**). Jede Antwort zitiert Stellen, die zum Transkript und Audio springen. Findet die
  App nichts, sagt sie das. **Recipes** (Eingabe „/“) sind gespeicherte Fragen mit Variablen wie Person,
  Ordner oder Zeitraum, etwa „Offene Aufgaben von … seit …“.
- Antwortet ein **externer** Anbieter, steht das in einer Leiste und Sie bestätigen einmal je Anbieter.

## Kalender und Erkennung

- **Kalender verbinden** (Einstellungen → Diktat → Besprechungen): ICS-Adresse von Outlook, Google, iCloud oder
  Nextcloud einfügen; der Dialog erklärt, wo Sie sie finden. Die Adresse wird verschlüsselt (Windows-Benutzerkonto)
  gespeichert. Termine kommen alle 15 Minuten und liefern Titel und Teilnehmende.
- **Als Nächstes** (Projekte-Spalte) zeigt den nächsten Termin mit „Termin aufnehmen“.
- Eine Minute vor Beginn erscheint ein **Hinweisfenster** mit „Aufnahme starten“, nur bei Besprechungen
  (ab zwei Teilnehmenden oder mit Beitritts-Adresse) und nie während einer Aufnahme.
- **Laufende Besprechungen erkennen**: Nutzt ein Programm das Mikrofon (Teams, Zoom, Webex, Browser …),
  fragt die App, ob sie aufnehmen soll. Sie startet nie von selbst; gelesen wird nur das Windows-Nutzungsprotokoll.

## Export und Follow-up

- **Exportieren** (Symbol in der Bedienung): Word, Text, Markdown, HTML, PDF, SRT, VTT oder JSON. Sie wählen die Teile (Notizen,
  Transkript …). Untertitel enthalten nur das Transkript; Audio wird nie exportiert. **Formatiert kopieren**
  legt HTML und Klartext in die Zwischenablage.
- **Follow-up-Mail**: Die App schreibt einen Entwurf. Sie prüfen ihn und wählen **Kopieren**, **Im Mailprogramm
  öffnen** oder **Als .eml speichern**. Versendet wird nie automatisch.

## MCP (in Arbeit)

Ein lokaler, nur lesender MCP-Server soll Claude, Codex und ähnlichen Werkzeugen Zugriff auf Ihre
Besprechungen geben. Er ist im Entwurf **standardmäßig aus** und warnt beim Einschalten, weil Inhalte dann an den
Anbieter des KI-Werkzeugs gehen. Dieser Stand enthält ihn noch nicht.

## Datenschutz

Aufnahme, Transkription, Sprechertrennung, Suche, Chat und Export laufen **lokal**. Nichts geht an einen
Dienst der App; es gibt keine Telemetrie. Netzverkehr entsteht nur, wenn Sie ihn auslösen:

| Anlass | Was geht raus |
|---|---|
| Modell laden (Transkription, Suche, Sprecher, Sprachmodell) | Download von Hugging Face bzw. dem Katalog-Spiegel blob.handy.computer, keine Inhalte |
| Kalender verbunden | Abruf der ICS-Adresse beim Kalenderanbieter |
| Externer Sprachmodell-Anbieter gewählt | Auszüge für KI-Notizen, Chat oder Follow-up, mit Hinweis vorab |

Mit einem lokalen Sprachmodell bleibt alles auf dem Rechner. **Aufbewahrung** der Audiodatei: 3, 14 oder 90 Tage,
unbegrenzt oder bis das Protokoll steht (Einstellung „Aufbewahrung der Audiodatei“). Transkript und Notizen
bleiben bis zum Löschen der Besprechung.
