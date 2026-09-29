# Besprechungen aufnehmen, ordnen und wiederfinden

Kurzanleitung für den Bereich **Besprechungen** in der Seitenleiste (Stand 29.09.2026).
Alles läuft auf diesem Rechner, ohne Konto, ohne Abo und ohne Bot im Videocall. Was
noch nicht geht, steht in [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md#besprechungen-stand-2026-09-29).

## Aufnehmen und mitschreiben

1. Titel eintragen (oder einen Termin aus dem Kalender wählen, siehe unten) und **Aufnahme starten**.
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

Während einer Aufnahme ist das Diktat gesperrt. Dateien (Audio, Video, VTT, SRT) lassen sich über
**Importieren …** hinzufügen und wie eine Aufnahme behandeln.

## KI-Notizen und Vorlagen

Nach dem Stopp macht die KI aus Ihren Stichpunkten und dem Transkript geordnete Notizen
(Reiter **Notizen**). Automatisch nur, wenn ein Sprachmodell eingerichtet ist (Einstellungen → Sprachmodelle);
sonst per **KI-Notizen erzeugen**.

- **Ihr Text bleibt Ihr Text**: Er wird nie verändert und ist vom KI-Text (grau) unterscheidbar.
- **Jede KI-Aussage hat einen Beleg.** Ein Klick auf die Quelle springt ins Transkript und ins Audio.
  Einträge „ohne Beleg“ sind markiert und wollen selbst geprüft werden.
- Klicken zum Bearbeiten; danach zählt der Eintrag als Ihr Text. Mit einer Anweisung (z. B. „kürzer“)
  lassen sich die Notizen umschreiben. Aufgaben stehen als Checkliste da.
- **Vorlagen** legen die Abschnitte fest. Mitgeliefert sind acht deutsche (Allgemein, Kundengespräch, Jour fixe,
  Projekt-Kickoff, Interview, Workshop, Lenkungskreis). **Vorlagen verwalten …**: eigene anlegen, duplizieren,
  als Datei (`.lvtemplate.json`) austauschen. Die Standardvorlage wählen Sie in den Einstellungen.

## Sprecher

Nach dem Stopp, beim Import und bei der Neu-Transkription trennt ein lokales Modell die Sprecher je Kanal
(bis zu vier) und beschriftet sie „Gegenseite 1“, „Person 2“ usw. Im Transkript klicken Sie auf den
Sprecher, um ihn zu **benennen**, mit einem anderen **zusammenzuführen** oder **nur ein Segment** umzuhängen. Namen
überstehen eine Neu-Transkription. Sitzen mehrere Personen am selben Mikrofon, aktivieren Sie
**Mehrere Personen am Mikrofon**. Ausschalten: Einstellungen → Diktat → Besprechungen → Sprecher automatisch trennen.
Das Modell (NVIDIA Sortformer) wird bei Bedarf geladen; seine Lizenz steht unter Info → Danksagungen.

## Chat und Suche

- **Suche** (Kopf der Liste): Stichworte über alle Besprechungen, Filter nach Zeitraum, Quelle und „Mit Notizen“.
  Mit dem Suchmodell **BGE-M3** (einmalig 635 MB, Einstellungen → Besprechungen → Semantische Suche) findet sie
  auch nach Bedeutung.
- **Ordner**: Eine Besprechung kann in mehreren Ordnern liegen. Löschen eines Ordners löscht keine Besprechung.
- **Fragen** (Strg+J) öffnet den Chat zu einer Besprechung, auch während der Aufnahme, oder über alle
  Besprechungen und Ordner. Jede Antwort zitiert Stellen, die zum Transkript und Audio springen. Findet die
  App nichts, sagt sie das. **Recipes** (Eingabe „/“) sind gespeicherte Fragen mit Variablen wie Person,
  Ordner oder Zeitraum, etwa „Offene Aufgaben von … seit …“.
- Antwortet ein **externer** Anbieter, steht das in einer Leiste und Sie bestätigen einmal je Anbieter.

## Kalender und Erkennung

- **Kalender verbinden** (Einstellungen → Diktat → Besprechungen): ICS-Adresse von Outlook, Google, iCloud oder
  Nextcloud einfügen; der Dialog erklärt, wo Sie sie finden. Die Adresse wird verschlüsselt (Windows-Benutzerkonto)
  gespeichert. Termine kommen alle 15 Minuten und liefern Titel und Teilnehmende.
- Eine Minute vor Beginn erscheint ein **Hinweisfenster** mit „Aufnahme starten“, nur bei Besprechungen
  (ab zwei Teilnehmenden oder mit Beitritts-Adresse) und nie während einer Aufnahme.
- **Laufende Besprechungen erkennen**: Nutzt ein Programm das Mikrofon (Teams, Zoom, Webex, Browser …),
  fragt die App, ob sie aufnehmen soll. Sie startet nie von selbst; gelesen wird nur das Windows-Nutzungsprotokoll.

## Export und Follow-up

- **Exportieren**: Word, Text, Markdown, HTML, PDF, SRT, VTT oder JSON. Sie wählen die Teile (Notizen,
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
