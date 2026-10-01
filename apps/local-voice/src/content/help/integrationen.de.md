# Integrationen

Verbindungen der App zu Kalendern, Ordnern, Postfach, Wissensbasis und Programmen, jede mit einer **Richtung** und einem **Recht je Fähigkeit**. Zugangsdaten bleiben auf diesem Rechner.

## Verbindungen und Rechte

- **Integration hinzufügen** bietet Kalender (ICS, Microsoft 365), Ordner, Postfach, Obsidian-Vault, Wissensbasis, YouTube und Webhooks (etwa n8n) an.
- Jede Fähigkeit (Mail senden, Dateien schreiben, Aufnahme starten …) hat ein Recht je Aufrufer: **Aus**, **Fragen** oder **Erlaubt**. Bei **Fragen** entscheidest du in der App jedes Mal. Schreibendes steht standardmäßig auf Fragen, externe Agenten sind aus.
- Die Rechte-Matrix zeigt, was jetzt wirklich wirkt. Eine ausgeschaltete Integration, eine falsche Richtung oder eine nicht angebotene Fähigkeit sperrt trotz Recht; der Grund steht dabei.
- Aufnahmen starten nie von allein: vor jeder Aufnahme erscheint der Einwilligungsdialog („Alle Beteiligten haben zugestimmt“). „Erlaubt“ gibt es dafür nicht.
- **Freigaben warten** erscheint oben, wenn ein Ablauf oder Agent etwas möchte, das auf „Fragen“ steht. Eine Freigabe gilt einmal, für genau diese Argumente.

## Agenten anbinden

- Unter **MCP und Agenten** läuft die **Agentenbrücke**. Sie ist nur für dich erreichbar (aktueller Windows-Benutzer), nicht über das Netzwerk.
- **Zugang anlegen** erzeugt je Programm (Claude Code, Codex, ein Skript) einen eigenen Schlüssel. Er wird nur einmal angezeigt; trage ihn als Umgebungsvariable `LVA_AGENT_TOKEN` ein, nie in eine Datei im Projekt. Zurückziehen gilt sofort.
- Je Zugang stellst du jedes Werkzeug auf **Aus**, **Fragen** oder **Erlaubt**. Alles ist zunächst aus. Die fertigen Anbindungsbefehle für Claude Code, Codex und die Kommandozeile `ctl` stehen auf der Seite; Einzelheiten in `docs/AGENTEN-ANBINDEN.md`.
- Inhalte aus Besprechungen können Anweisungen enthalten, die nicht von dir stammen. Agenten sollen sie nie als Befehl behandeln.

## Automationen

Der Reiter **Automationen** hält deine **Abläufe**: ein Auslöser (Termin, Datei im Ordner, neues YouTube-Video, fertige Besprechung, Zeitplan, von Hand oder durch einen Agenten) und Schritte danach.

- **Neuer Ablauf** beginnt mit einer Vorlage oder leer; **JSON importieren/exportieren** tauscht Abläufe ohne Zugangsdaten aus.
- Ein neuer oder geänderter Ablauf ist **ausgeschaltet** und im **Trockenlauf**: **Trockenlauf anzeigen** zeigt je Schritt, was er täte und ob er dürfte. Es wird nichts geschrieben, gesendet oder aufgenommen. Erst **Scharf schalten** lässt Schritte wirken.
- Die Schritte laufen mit den Rechten des Ablaufs („Workflow“): Mails, Dateien, Webhook und Aufnahme fragen, wenn das Recht auf „Fragen“ steht. Ein Lauf, der auf deine Freigabe wartet, zeigt „Freigabe prüfen“.
- **Läufe** listet jeden Lauf mit Schritten, Fehlern und Herkunft; Abbrechen und Wiederholen sind dort möglich.

## Protokoll

Der Reiter **Protokoll** zeigt jede Aktion eines Ablaufs oder Agenten und jede Rechteänderung, filterbar nach Integration, Ergebnis und Aufrufer.
