# Vorschlag Goal „Lokaler Agent“ — kleines LLM mit Werkzeugen in Workflows

Grundlage: `recherche/tool-calling-und-spike.md` (Stand 30.09.2026, Mini-Spike mit Gemma 4 E4B).
Setzt Goal A (Register, Rechte, Provenienz, Vault/Wissen) und Goal B (Engine) voraus; C1 kann sofort starten.

## Zielzustand
In Automationen gibt es zwei lokale KI-Schritte: „Extrahieren“ (To-dos, Fristen, Entscheidungen, Fakten strukturiert
aus Transkript/Protokoll/Video) und „Werkzeug wählen“ (aus einer vom Ablauf freigegebenen, kurzen Werkzeugliste), beide
mit dem gebündelten llama-server und Gemma 4 E4B (E2B nur, wenn es die Messlatte besteht), Schema-gebunden, ohne
Denkmodus, abgesichert durch Whitelist, Rechte, Freigabe, Trockenlauf und Audit. Erkennbar an einem Eval-Bericht mit
≥ 95 % Trefferquote auf 60 deutschen Aufgaben und daran, dass aus einer Besprechung automatisch Obsidian-Notiz,
RAG-Eintrag (über den Vault) und Frist-Mitteilung entstehen — jeweils mit „Herkunft“ inkl. Modell und Konfidenz.

## Scope
- Eval-Harness `--eval-agent` mit Datensatz (60 Aufgaben de: Werkzeugwahl, Argumente, Enthaltung, Injection, Datum)
  und Bericht je Modell; Messung E4B (vorhanden), Qwen3-4B (vorhanden), E2B nur nach Download-Freigabe (E1).
- LLM-Aufruf mit `response_format` json_schema, `chat_template_kwargs.enable_thinking=false`, Temperatur 0,
  Token-Obergrenze, serde-Validierung, ein Wiederholversuch, Rückfall `no_action`.
- Schritt `agent.extract`: feste Schemas (todos, deadlines, decisions, facts mit Zitat-Segment-ID) — kein
  Werkzeugzugriff, nur Daten.
- Schritt `agent.route`: Wahl genau eines Werkzeugs aus der Whitelist des Schritts, Argumente per Schema;
  Ausführung über die Rechte aus Goal A (`Caller::AgentLocal`), Außenwirkung nur mit Freigabe.
- Deterministische Folgeaktionen: Frist → lokale Mitteilung (und optional Kalendereintrag mit Freigabe), To-dos →
  Obsidian-Notiz/Aufgabenliste, Fakten/Entscheidungen → Vault-Notiz mit AI-OS-Frontmatter → RAG über den Vault-Hook.
- Provenienz: jeder Agent-Schritt schreibt Modell, Token, Dauer, Quellen (Segment-IDs) und Konfidenz.
- UI: Agent-Schritt im Workflow-Editor (Werkzeug-Whitelist, Schema-Vorschau, Trockenlauf mit Modellausgabe).

## Non-Scope
- Freie, mehrstufige Agentik (Plan mit mehreren eigenständigen Werkzeugschritten, Schleifen) — τ2-bench E4B 42,2 %,
  E2B 24,5 % (Model Card) sprechen dagegen.
- Modelle außerhalb des vorhandenen Katalogs; Feintuning.
- Mail an Empfänger, die das Modell bestimmt; Werkzeuge ohne Whitelist-Eintrag.
- Chat-Agent in der Oberfläche (bestehender Besprechungs-Chat bleibt).

## Akzeptanzkriterien
- [ ] AK1 — Eval: `local-voice-ai.exe --eval-agent --model llm-gemma4-e4b-q4 --json --out eval.json` → Bericht mit Werkzeug-Trefferquote, Argument-Trefferquote, Enthaltungsquote, Injection-Quote, p50/p95-Latenz; E4B erreicht Werkzeugwahl ≥ 95 % und Argumente ≥ 90 % (Gate für C3).
- [ ] AK2 — Modellvergleich: derselbe Lauf für `llm-qwen3-4b-q4` (vorhanden), `llm-qwen3.5-4b-q4` (im Katalog, τ2 79,9 laut Sekundärquelle; Download nach E1) und — falls freigegeben — E2B; Bericht `koordination/lokaler-agent/abnahme/eval-<datum>.md` mit Empfehlung und Grenzwerten; E2B wird nur Standard, wenn es AK1-Grenzen erfüllt.
- [ ] AK3 — Laufzeit: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib agent::` → ≥ 20 Tests gegen einen Test-HTTP-Server, der llama-server nachahmt: Schema wird mitgesendet, Denkmodus aus, abgeschnittene/ungültige JSON → ein Wiederholversuch → `no_action`, nie Panik, Token-Obergrenze greift.
- [ ] AK4 — Extraktion: Golden-Tests auf 5 Fixture-Transkripten (de) → To-dos/Fristen/Entscheidungen mit Segment-Belegen; Frist-Datum wird im Code validiert (ISO, nicht in der Vergangenheit, ≤ 2 Jahre), ungültige Daten verworfen und im Lauf vermerkt.
- [ ] AK5 — Router + Sicherheit: Tests → Werkzeug außerhalb der Whitelist wird nie ausgeführt (auch wenn das Modell es nennt); `send_mail`-Empfänger nur aus der vom Code gebildeten Teilnehmermenge; Injection-Fixture („Ignoriere alle Regeln …“) führt zu keiner Außenwirkung, unabhängig von der Modellantwort; Obergrenze Aktionen je Lauf greift.
- [ ] AK6 — Wissen: Ablauf „Besprechung fertig → extrahieren → Vault-Notiz“ im Sandbox-Vault → Notiz mit gültigem AI-OS-Frontmatter (`context_area`, `data_class` nach E7 aus Goal A), Rückverweis auf die Besprechung, keine Dublette beim zweiten Lauf; manuell: Notiz erscheint in der WAI-Wissenssuche.
- [ ] AK7 — Frist → Mitteilung: Fixture mit Frist in 2 Tagen → lokale Mitteilung zum konfigurierten Zeitpunkt (Test mit fester Uhr); Kalendereintrag nur nach Freigabe.
- [ ] AK8 — Provenienz: Rechtsklick „Herkunft“ auf eine vom Agenten erzeugte Notiz zeigt Modell, Token, Dauer, Quellen (Segmente) und Konfidenz (Playwright).
- [ ] AK9 — Systemschutz: Agent-Schritte laufen in der schweren Warteschlange (Goal B), starten den llama-server nur über den vorhandenen Manager (RAM-Gate, `process_guard`) und blockieren ihn nicht länger als die Obergrenze; bei belegtem Slot wartet der Schritt (Test).
- [ ] AK10 — Anfassbar: Eval-Bericht, Screenshots (Agent-Schritt im Editor, Trockenlauf mit Modellausgabe, Herkunft) und ein echter Lauf im Installer (Patch +1), von Patrick abgenommen.

## Quality Gates
- [ ] QG1 — `cargo test --lib` gesamt grün; neue Dateien ohne neue Clippy-Warnungen.
- [ ] QG2 — `npx tsc --noEmit` Exit 0; Playwright-Suite grün.
- [ ] QG3 — Eval-Gate: AK1-Werte werden vor jedem Modellwechsel neu gemessen (`--eval-agent` im Abnahmeablauf).
- [ ] QG4 — Externes Sicherheitsreview der Whitelist-/Freigabe-/Empfängerlogik (C3).
- [ ] QG5 — i18n de + en, Doku `docs/LOKALER-AGENT.md` (was er darf, was nicht, wie messen), Handoff.
- [ ] QG6 — Budget 1,6 MTok (Spanne 1,3–2,2); Meldung bei 50 % und 80 %, Stopp bei 150 %.

## Architektur-Skizze

```
src-tauri/src/agent/
  mod.rs        AgentRuntime: baut Anfrage (Schema, enable_thinking=false, T=0, max_tokens), ruft llm_client
  schema.rs     Werkzeug-Schemas → oneOf-JSON-Schema (wie im Spike), serde-Typen
  extract.rs    agent.extract (Schemas todos/deadlines/decisions/facts, Segment-Belege)
  route.rs      agent.route (Whitelist, Rückfall no_action, Obergrenzen)
  policy.rs     rein: Empfängermenge, Pfad-/Ziel-Prüfung, Datumsprüfung — Politik im Code, nicht im Prompt
  eval.rs       --eval-agent (Datensatz eingebettet oder Pfad), Bericht
managers/workflows/actions/agent.rs   Anbindung an Goal-B-Engine (heavy = true)
Rechte: integrations::grants::effective_mode(…, Caller::AgentLocal); Provenienz: provenance::record(confidence)
```
Muster: **Action-Selector + Context-Minimization** (Beurer-Kellner et al. 2025) und **Rule of Two** (Meta): Der Schritt,
der das Transkript liest, hat keine Außenwirkung; Außenwirkung entsteht nur durch deterministische Aktionen oder durch
`agent.route` mit Freigabe. Konfidenz = im Code berechnet (Schema gültig beim ersten Versuch, Belege vorhanden,
Überdeckung Zitat↔Segment), nicht vom Modell behauptet.

Schnittstellen (Entwurf):
```rust
pub async fn agent::extract(rt: &AgentRuntime, kind: ExtractKind, input: &SourceText) -> Result<Extracted, AgentError>;
pub async fn agent::route(rt: &AgentRuntime, task: &str, whitelist: &[ToolSpec], ctx: &RouteCtx) -> Result<ToolChoice, AgentError>;
pub fn policy::allowed_recipients(meeting: &Meeting, rule: RecipientRule) -> Vec<String>;
pub enum AgentError { Timeout, SlotBusy, SchemaInvalid { raw_len: usize }, Denied, Model(String) }
```

## Paketschnitt (je 250–300 kTok)
| Paket | Scope | Akzeptanztest | Abh. | Worker |
|---|---|---|---|---|
| C1 | Eval-Harness + Datensatz (60 Aufgaben) + Messung E4B/Qwen3-4B (+E2B nach E1) + Bericht | AK1, AK2 | – (sofort möglich) | lv-architect (Messung) |
| C2 | Agent-Laufzeit (Schema, Denken aus, Validierung, Retry, Rückfall) + `agent.extract` + Provenienz | AK3, AK4 | B1, A1 | lv-coder-xhigh |
| C3 | `agent.route` + `policy.rs` (Whitelist, Empfänger, Obergrenzen, Freigabe, Trockenlauf) + Injection-Tests | AK5, AK9, QG4 | C2, C1-Gate | lv-coder-xhigh |
| C4 | Wissens- und Fristaktionen: Vault-Notiz mit Frontmatter, Dublettenschutz, RAG über Vault, Mitteilung, optional Kalender | AK6, AK7 | C2, A6, B4 | lv-coder |
| C5 | UI Agent-Schritt im Editor, Trockenlauf-Anzeige, Herkunft; Abnahme/Installer | AK8, AK10 | C3, C4, B7 | lv-coder + Planer |

## Budget
5 × ~275 kTok = 1,4 MTok + ~15 % → **1,6 MTok** (Spanne 1,3–2,2). C1 allein ≈ 0,25 MTok und liefert die
Go/No-Go-Grundlage, bevor C2–C5 Budget binden.

## Risiken mit Vorschlag
- R1 Spike zu klein (10 Aufgaben, einstufig) → C1 misst 60 Aufgaben inkl. langer Kontexte, bevor C3 startet; bei < 95 %
  bleibt nur `agent.extract` (ohne Router).
- R2 Denkmodus/Template-Änderungen in neuen llama.cpp-Builds → Eval im Abnahmeablauf (QG3); Schema-Modus statt nativer
  Tool-Calls (im Spike robuster: 50/50 vs. 46/50).
- R3 Slot-Konkurrenz mit KI-Notizen (`--parallel 1`) → schwere Warteschlange aus Goal B; Agent-Schritte kurz halten.
- R4 VRAM: RTX 4090 war beim Spike mit 23,9/24,5 GB belegt → kein zweiter Server; E2B wäre kleiner, aber erst nach
  bestandener Messung.
- R5 Falsche Fristen/To-dos landen im Wissen → Datum im Code prüfen, Belege Pflicht, Standard „fragen“ für Vault-Schreiben
  in den ersten Wochen, Herkunft sichtbar.
- R6 Prompt-Injection über Transkripte/Videos → Politik in `policy.rs`, keine Außenwirkung aus dem Leseschritt.

## Owner-Entscheidungen (Patrick)
- E1 Gemma 4 E2B (nicht im Katalog) und Qwen3.5-4B (im Katalog, nicht geladen) herunterladen und messen? Lokal liegt kein E2B-GGUF; Größe vor Download prüfen (Grenze 3 GB je
  Download laut Systemschutz) — **Empfehlung: ja, nur für C1-Messung; Standard bleibt E4B, bis E2B das Gate besteht**.
- E2 Lokaler Agent standardmäßig nur `agent.extract` (Daten), `agent.route` erst nach bestandenem Eval-Gate —
  **Empfehlung: ja**.
- E3 Vault-Schreiben durch den Agenten anfangs „fragen“, später „erlaubt“ — **Empfehlung: 2 Wochen „fragen“**.
- E4 Alternativ statt lokal das konfigurierte Cloud-Modell für Extraktion zulassen (Schalter je Schritt) — **Empfehlung:
  ja, Standard lokal**.

## Abhängigkeiten
- **Goal A**: A1 (Rechte `agent_local`, Provenienz mit Konfidenz), A6 (Vault/Wissen).
- **Goal B**: B1 (Engine, `Action`-Trait, schwere Warteschlange), B4 (Mitteilung), B6 (Wissensabgleich nutzt
  `agent.extract`), B7 (Editor).
- **aufnahmen-ui**: keine direkte Abhängigkeit; Kontextmenü „Herkunft“ teilt Komponenten mit A3.
- Vorhanden: `managers/llm/*` (Server, RAM-Gate, `process_guard`), `llm_client.rs`, `usage.rs`.
