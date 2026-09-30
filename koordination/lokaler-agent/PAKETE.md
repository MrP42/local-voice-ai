# Pakete — Lokaler Agent: kleines LLM (Gemma 4 E2B) mit Werkzeugen in Workflows (Goal: C:/Users/wolff/local-voice-project/.claude/worktrees/wt-plan/koordination/lokaler-agent/GOAL.md)

Status: offen | in_arbeit | geliefert | abgenommen | nacharbeit | abgebrochen.
**Nur der Planer schreibt diese Datei** (Zuweisung, Status, Commit). Worker bearbeiten ausschließlich
das ihnen zugewiesene Paket und melden `STATUS: DONE | PARTIAL | BLOCKED` im Report.
Nacharbeit = neues Paket mit Bezug (z. B. P2n), kein Rücksprung. Jede Zeile beginnt mit `| P`.

| ID | M | Paket | Abnahmekriterium | Status | Commit |
|---|---|---|---|---|---|

## Paketschnitt (Vorschlag)
| Paket | Scope | Akzeptanztest | Abh. | Worker |
|---|---|---|---|---|
| C1 | Eval-Harness + Datensatz (60 Aufgaben) + Messung E4B/Qwen3-4B (+E2B nach E1) + Bericht | AK1, AK2 | – (sofort möglich) | lv-architect (Messung) |
| C2 | Agent-Laufzeit (Schema, Denken aus, Validierung, Retry, Rückfall) + `agent.extract` + Provenienz | AK3, AK4 | B1, A1 | lv-coder-xhigh |
| C3 | `agent.route` + `policy.rs` (Whitelist, Empfänger, Obergrenzen, Freigabe, Trockenlauf) + Injection-Tests | AK5, AK9, QG4 | C2, C1-Gate | lv-coder-xhigh |
| C4 | Wissens- und Fristaktionen: Vault-Notiz mit Frontmatter, Dublettenschutz, RAG über Vault, Mitteilung, optional Kalender | AK6, AK7 | C2, A6, B4 | lv-coder |
| C5 | UI Agent-Schritt im Editor, Trockenlauf-Anzeige, Herkunft; Abnahme/Installer | AK8, AK10 | C3, C4, B7 | lv-coder + Planer |
