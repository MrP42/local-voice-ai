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

## B3 — Merge A2+U5: search::index unfiled_filter-Test einmal rot im Gesamtlauf (30.09.)
- Beobachtet: `unfiled_filter_lists_meetings_without_a_living_folder` 1x rot in voller Suite, danach 2x Suite grün, einzeln 3x grün.
- Konsequenz: beobachten; beim zweiten Auftreten Ursache (Zeit/Isolation) beheben.
- Status: erledigt (beobachtet)

## B4 — A3: axe meldet zu geringen Farbkontrast (abgeblendete Schrift text-text/60) app-weit (30.09.)
- Beobachtet: axe `color-contrast` „serious“ bei Zeitmarken, Spaltenüberschriften, Leerhinweisen, Navigation; Ursache Design-Token, betrifft die ganze App.
- Konsequenz: eigenes Paket (Token-Anpassung app-weit) mit Owner-Freigabe wegen breitem Diff; bis dahin axe ohne color-contrast.
- Status: offen

## B5 — Zeitabhängige Tests zum zweiten Mal rot: search::index unfiled_filter, llm::server restart cooldown (30.09.)
- Beobachtet: beide erneut in vollen Läufen rot, einzeln grün (B3 war das erste Auftreten).
- Konsequenz: Regel „beim zweiten Auftreten reparieren“ – im Integrationspaket I1 ursachengerecht lastfest machen.
- Status: offen

