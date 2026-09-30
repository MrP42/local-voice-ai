# Befunde des Planers — Integrationen: Register fuer Kalender, Mail, Speicher, Wissen und Agenten (MCP/CLI lesend+schreibend)

(Je Befund: Überschrift `## B<n> — <Paket>: <Titel>`, Zeilen Beobachtung / Beleg / Konsequenz und
eine Zeile `- Status: offen` bzw. `- Status: erledigt (<Paket/Commit>)`. Offene Befunde verhindern COMPLETE.)

## B1 — A1: Sicherheitskern braucht externes Review vor Bündel 2 (30.09.)
- Beobachtet: Tor (gate.rs) ist kooperativ, approvals::decide ohne Aufruferprüfung, Audit ohne Manipulationsschutz, MCP-Lesewerkzeuge noch nicht über Grants gesteuert.
- Konsequenz: Codex-Review (Gate-/Sicherheitslogik) vor A4/A7; Adapter A5–A7 müssen gate::run verwenden (Test je Adapter); Freigabe-Entscheidung nur aus der UI, nie über die Agentenpipe.
- Status: offen

## B2 — Review A1 (30.09.): 6 Härtungspunkte im Rechtekern
- Beobachtet (feature-dev:code-reviewer, keine kritischen Funde): (1) Freigabe-Vorschau kann Empfänger abschneiden; (2) Audit-Flutung durch deny; (3) Freigabe-Flutung ohne Deduplizierung; (4) ungeprüfte IDs im Audit; (5) Lücken der Geheimnis-Erkennung; (6) Trigger ohne kind-Filter; dazu store::list-Abbruch, Provenienz-Schwärzung.
- Konsequenz: Paket A1n.
- Status: erledigt (A1n 262210d3, je Fund rot-vor-grün-Test)

