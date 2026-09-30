# Pakete — Integrationen: Register fuer Kalender, Mail, Speicher, Wissen und Agenten (MCP/CLI lesend+schreibend) (Goal: C:/Users/wolff/local-voice-project/.claude/worktrees/wt-plan/koordination/integrationen/GOAL.md)

Status: offen | in_arbeit | geliefert | abgenommen | nacharbeit | abgebrochen.
**Nur der Planer schreibt diese Datei** (Zuweisung, Status, Commit). Worker bearbeiten ausschließlich
das ihnen zugewiesene Paket und melden `STATUS: DONE | PARTIAL | BLOCKED` im Report.
Nacharbeit = neues Paket mit Bezug (z. B. P2n), kein Rücksprung. Jede Zeile beginnt mit `| P`.

| ID | M | Paket | Abnahmekriterium | Status | Commit |
|---|---|---|---|---|---|
| A1 | 1 | Register-Kern + Provenienz (Backend) — lv-coder-xhigh | AK1: --lib integrations:: provenance:: >= 30 Tests, Migration idempotent, Provenienz an Protokoll/KI-Notizen/Follow-up/STT | abgenommen | 20f0e763 |
| A1n | 1 | Härtung Rechtekern nach Review (B2) — lv-coder-xhigh | je Fund ein Test, integrations::/provenance:: + volle Suite grün | abgenommen | 262210d3 |
| A2 | 1 | YouTube-Quelle: Link-Normalisierung (watch, youtu.be, shorts), oEmbed-Metadaten, Besprechung mit Quelle youtube im gewählten Projekt, offizieller eingebetteter Player in der Inhaltsspalte mit Zeitsprung über transcriptPlayer, Schalter „privat“ für selbst installiertes yt-dlp (Standard aus), Link-Knopf aktiv — lv-coder-xhigh | AK2, AK3 (Playwright youtube-source; Player-Sprung) | abgenommen | 17aa79c0 |
| A3 | 1 | Untertitel (yt-dlp privat) + eigene Transkription (Audio per yt-dlp) als Fassungen, Diff-Ansicht, Fassung wählen, KI-Zusammenführen, Zusammenfassung, Rechtsklick „Herkunft“, Dauer aus Player, axe-Prüfung (devDependency) — lv-coder-xhigh | AK4, AK5, AK6 | abgenommen | 2f0c7642 |

## Paketschnitt (Vorschlag)
| Paket | Bündel | Scope | Akzeptanztest | Abh. | Worker |
|---|---|---|---|---|---|
| A1 | 1 | Fundament: Register-Kern (Migration, Kalender-Übernahme, Grants, Audit, Approvals, Geheimnis-Namensraum, `--integrations-dump`) + Provenienz (Tabelle, API, `Purpose`-Erweiterung, Einbau in Protokoll/KI-Notizen/Zusammenfassung/STT) | AK1 | – | lv-coder-xhigh |
| A2 | 1 | YouTube-Quelle: Link-Normalisierung, oEmbed, Besprechung mit Quelle `youtube`, eingebetteter Player mit Segment-Sprung, Adapter für Dateiweg nach E1, minimale YouTube-Karte im Register | AK2, AK3 | A1, aufnahmen-ui M2 | lv-coder |
| A3 | 1 | Untertitel + Fassungen + Diff + Zusammenführen + Zusammenfassung + Kontextmenü „Herkunft“; Installer Bündel 1 | AK4, AK5, AK6, AK12 (1) | A2 | lv-coder-xhigh |
| A4 | 2 | Seite Integrationen (Liste, Katalog, Detail, Rechte-Matrix, Audit-Ansicht, Freigabedialog), Umzug Kalender/MCP | AK7 | A1 | lv-coder |
| A5 | 2 | Microsoft-365-Konto: Scopes je Fähigkeit, Mail senden, OneDrive, Termin-Notiz; Follow-up-Mail „senden über“ | AK8 (m365) | A1 | lv-coder-xhigh |
| A6 | 2 | SMTP, Ordner (Sandbox), Obsidian-Vault, WAI-Wissensbasis; Export „ablegen in“ | AK8 (Rest) | A1, A4 | lv-coder |
| A7 | 2 | Agentenbrücke: Named Pipe, Client-Token, Werkzeug-Rechte, Freigaben, `ctl`-CLI | AK9 | A1 | lv-coder-xhigh |
| A8 | 2 | MCP schreibend (inkl. `add_youtube_source`, `get_provenance`), Protokollversion 2026-07-28 prüfen, `mcp_smoke.py --write`, Audit-Dump; Doku, Sicherheitsreview, Installer Bündel 2 | AK10, AK11, AK12 (2) | A7, aufnahmen-ui M3 | lv-coder-xhigh + Planer |
| A9 | später | Playlist: Links auflösen, je Video eine Besprechung in einer Session, Fortschritt | – | A3 | – (nicht im Budget) |
