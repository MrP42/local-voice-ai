# Pakete — Workflow-Automation: Besprechungen, Ordner und Integrationen automatisieren (Goal: C:/Users/wolff/local-voice-project/.claude/worktrees/wt-plan/koordination/workflow-automation/GOAL.md)

Status: offen | in_arbeit | geliefert | abgenommen | nacharbeit | abgebrochen.
**Nur der Planer schreibt diese Datei** (Zuweisung, Status, Commit). Worker bearbeiten ausschließlich
das ihnen zugewiesene Paket und melden `STATUS: DONE | PARTIAL | BLOCKED` im Report.
Nacharbeit = neues Paket mit Bezug (z. B. P2n), kein Rücksprung. Jede Zeile beginnt mit `| P`.

| ID | M | Paket | Abnahmekriterium | Status | Commit |
|---|---|---|---|---|---|

## Paketschnitt (Vorschlag)
| Paket | Scope | Akzeptanztest | Abh. | Worker |
|---|---|---|---|---|
| B1 | Engine-Kern: Modell, Schema, `expr`, Store, Warteschlange, Retry, Idempotenz, Wiederaufnahme, Provenienz-Anbindung, `--workflow-run --dry-run` | AK1, AK2, QG5 | A1 | lv-coder-xhigh |
| B2 | Auslöser Kalender, Besprechungsereignisse, Zeitplan, manuell; Einwilligungsweg | AK3, AK4 | B1 | lv-coder-xhigh |
| B3 | Auslöser Ordner (Debounce, Stabilität, Ledger, OneDrive-Platzhalter) und YouTube-Kanal (RSS, Ledger, Ausfallmeldung) + Aktion Import/Transkription | AK5, AK11 (Auslöser) | B1, A3 | lv-coder |
| B4 | App-Aktionen: Notizen, Protokoll, Zusammenfassung, Export in Ordner, TTS, lokale Mitteilung, Warten; Vorlage „Eingangsordner → Word“ | AK6 | B1, A6 | lv-coder |
| B5 | Integrations-Aktionen: Mail (Empfängerregeln, Freigabe), Termin-Notiz, Webhook (n8n); Vorlage „Termin → Mail“ | AK7, AK8 | B2, A5, A6 | lv-coder-xhigh |
| B6 | Wissens-Aktionen: Relevanz, Abgleich (neu/vorhanden/ergänzt/widerspricht), Vault-Schreiben ohne Dubletten, Kanal-Management-Summary; Vorlage „Kanal → Wissen“ | AK11 | B3, A6, (C2 optional) | lv-coder-xhigh |
| B7 | Oberfläche Automationen: Liste, Formular-Editor, Vorlagen, Lauf mit „Herkunft“, Freigaben, JSON-Import/Export | AK9 | B1, A4 | lv-coder |
| B8 | Agenten (MCP/CLI `list_workflows`/`run_workflow`/`get_run`), n8n-Brücke als Doku + Beispiel, Ende-zu-Ende aller drei Vorlagen, Sicherheitsreview, Installer | AK10, AK12, QG4, QG6 | alle | lv-coder + Planer |
| B9 | optional: Faktencheck (Websuche nach E7, Konfidenz je Aussage, Hinweise auf Falschbehauptungen) | – | B6 | – (nicht im Basisbudget) |
