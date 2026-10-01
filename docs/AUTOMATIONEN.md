# Automationen: Abläufe, Rechte, Trockenlauf

Stand 01.10.2026 (Version 0.21.0, Goal „Workflow-Automation“, Issue #67). Ein **Ablauf** führt auf einen
**Auslöser** hin eine Reihe von **Schritten** aus, etwa „Termin beginnt → aufnehmen → Protokoll → Mail“.
Zu finden unter **Integrationen → Automationen**; zum Anbinden von Agenten siehe
[AGENTEN-ANBINDEN.md](AGENTEN-ANBINDEN.md).

## Auslöser

| Auslöser | Startet |
|---|---|
| Termin beginnt / endet | Kalender (ICS oder Microsoft 365), mit Vorlaufzeit, Filter „nur Besprechungen“, Titel, Mindestzahl Teilnehmende |
| Besprechung fertig | wenn Transkript (oder Protokoll) einer Besprechung vorliegt |
| Datei im Ordner | neue Audio-/Videodatei in einem Ordner; sie muss einige Sekunden unverändert sein. OneDrive-Platzhalter („nur in der Cloud“) werden nie heruntergeladen und nicht verarbeitet |
| Neues Video im Kanal | YouTube-Kanal, Abruf alle n Minuten; beim ersten Abruf auf Wunsch die neuesten Videos nachholen; antwortet der Kanal nicht, wird nachgeholt, nichts geht verloren |
| Zeitplan | täglich oder wöchentlich (gewählte Wochentage) zu einer Uhrzeit |
| Von Hand / Durch einen Agenten | nur auf Zuruf: Knopf „Jetzt starten“ oder `run_workflow` über die Agentenbrücke |

## Vorlagen

**Neuer Ablauf** beginnt mit einer Vorlage (oder leer). Die Vorlage wird im Editor angepasst (Ordner, Kalender,
Konten wählen) und erst mit „Speichern“ angelegt:

- **Termin: aufzeichnen, Protokoll, Mail**: Aufnahme starten (nach deiner Einwilligung), Protokoll erzeugen, als Word
  ablegen, per Mail senden. Die Variable `empfaenger` (`ich` oder `alle`) bestimmt, wer die Mail bekommt.
- **Eingangsordner: transkribieren, Protokoll als Word**: Datei im Ordner → importieren und transkribieren → Protokoll → Word im Zielordner.
- **Kanal → Wissen**: neues YouTube-Video → Untertitel → Zusammenfassung → Relevanz nach deinem Themenprofil →
  Abgleich mit Wissensbasis und Vault → Notiz ohne Dublette. Bestehende Notizen ändert der Ablauf nur nach Freigabe.
- **Besprechung fertig: Ergebnis in den Vault, Fristen erinnern**: das lokale Sprachmodell zieht To-dos, Fristen und
  Entscheidungen mit Belegen heraus; Obsidian-Notiz, auf Wunsch Kalendereintrag je Frist (nur nach Freigabe) und eine
  Windows-Mitteilung am Vortag um 09:00 Uhr.

Abläufe lassen sich als JSON (`lva-workflow@1`) exportieren und importieren. Der Export enthält keine Zugangsdaten,
nur Kennungen aus dem Register der Integrationen; ein Import legt einen neuen Ablauf an, ausgeschaltet und im Trockenlauf.
Beispiele: [`beispiele/`](beispiele/).

## Rechte: Aus, Fragen, Erlaubt

Jeder Schritt, der etwas außerhalb der App bewirkt (Mail, Datei schreiben, Webhook, Aufnahme, Notiz in Vault oder Termin),
braucht ein **Recht** der Integration, über die er läuft. Gelten tut das Recht des Aufrufers **Workflow**:

| Recht | Wirkung |
|---|---|
| **Aus** | der Schritt wird abgelehnt (im Plan: „abgelehnt“) |
| **Fragen** | der Lauf hält an und wartet auf deine Freigabe in der App („Freigabe prüfen“); die Freigabe gilt einmal, für genau diese Parameter |
| **Erlaubt** | der Schritt läuft ohne Rückfrage |

Dazu zählen die **Obergrenze** der Integration (ausgeschaltet, Richtung, nicht angebotene Fähigkeit sperren trotz Recht) und
die Rechte-Matrix unter Integrationen. Schreibendes steht standardmäßig auf „Fragen“, nie auf „Erlaubt“. Ein Agent kann über
einen Ablauf nie mehr auslösen, als du dem Ablauf ohnehin erlaubt hast.

## Trockenlauf und Scharfschalten

- Ein neuer, importierter oder geänderter Ablauf ist **ausgeschaltet** und im **Trockenlauf**.
- **Trockenlauf anzeigen** berechnet den Plan: je Schritt, was er täte, mit welchen Parametern, ob das Recht reicht
  („erlaubt“, „fragt vorher“, „abgelehnt“), ob eine Bedingung den Schritt auslässt und wie viel Arbeitsspeicher schwere
  Arbeit braucht. Es wird nichts geschrieben, gesendet oder aufgenommen. Auslöserdaten sind dabei Beispieldaten.
- **Scharf schalten** erst, wenn der Plan stimmt; danach wirken die Schritte wirklich. Ein Lauf eines nicht scharfen
  Ablaufs plant nur. Wird ein scharfer Ablauf geändert, fällt er in den Trockenlauf zurück.
- Der Knopf **Probelauf starten** zeigt, wie ein Lauf aussieht; **Jetzt starten** gibt es nur für einen eingeschalteten,
  scharfen Ablauf.

## Einwilligung zur Aufnahme (§ 201 StGB)

Der Schritt „Aufnahme starten“ beginnt **nie von selbst**. Beim Auslösen erscheint das Hinweisfenster der Aufnahme mit
dem Häkchen „Alle Beteiligten haben zugestimmt“; erst nach dem Häkchen und „Aufnahme starten“ läuft sie, „Nicht aufnehmen“
lehnt ab. „Erlaubt“ gibt es für diese Fähigkeit nicht. Jede Aufnahme eines Ablaufs hat ein Ende (Vorgabe: längstens
die eingestellten Minuten), damit eine vergessene nicht die Platte füllt.

## Läufe, Fehler, Wiederholung

- **Läufe** zeigt jeden Lauf mit Zustand (wartet, läuft, wartet auf Freigabe, fertig, fehlgeschlagen, abgebrochen), Herkunft des
  Starts, Schritten, Eingaben und Ergebnissen sowie der **Herkunft** der KI-Schritte (Modell, lokal oder extern, Quellen).
- Je Schritt: Bedingung (läuft nur, wenn sie stimmt), **Versuche** und Wartezeit, und die Fehlerregel
  „Lauf beenden“ oder „Weitermachen“.
- Ein Lauf lässt sich abbrechen (er endet am nächsten Schrittwechsel). Bei einem Schritt, von dem unklar ist, ob er
  schon wirkte (etwa eine Mail), wiederholst du nur, wenn du bestätigst, dass er doppelt wirken darf.
- Ein Lauf überlebt einen Neustart der App; schwere Schritte (Transkription, Sprachmodell) laufen nacheinander und
  respektieren das Speicher-Tor: ist der Arbeitsspeicher knapp, wartet der Schritt, statt den Rechner zu belasten.

## Lokaler Agent im Ablauf

Zwei Bausteine nutzen das lokale Sprachmodell: **Werkzeug wählen** (das Modell wählt aus einer von dir freigegebenen Liste
höchstens n Aktionen) und **Aufgaben, Fristen, Entscheidungen extrahieren**. Das Modell entscheidet nur; die Empfänger
bildet das Programm aus deiner Regel, nie das Modell, und ausgeführt wird eine Wahl erst von einem folgenden Schritt mit
eigenem Recht und eigener Freigabe. Text aus Besprechungen gilt dabei als Daten, nie als Anweisung; erkannte
Aufforderungen an die KI im Text werden angezeigt. „Mit Beispieltext ausprobieren“ im Editor testet das ohne jede Wirkung.

## Grenzen

Siehe [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md#folien-ocr-workflows-und-integrationen-stand-2026-10-01): Meldungen des
Programmkerns erscheinen auch in der englischen Oberfläche deutsch; ein Webhook wird nie durch Senden getestet.
