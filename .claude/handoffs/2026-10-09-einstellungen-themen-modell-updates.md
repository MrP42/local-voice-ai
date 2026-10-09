# Handoff 09.10.2026: Einstellungen nach Thema, Fußleisten-Sprung, Modell-Updates (0.21.10)

## Stand
- Zweig `feat/einstellungen-themen` (lokal, **nicht gepusht**, kein PR), aufgesetzt auf `feat/protokoll-kopf-kompakt` (#81, Kette #76→#81).
- Commits: Spezifikation + Codex-Review → Teil 1 → Teil 2 → Teil 3 → Version 0.21.10.
- Spezifikation: `docs/superpowers/specs/2026-10-09-einstellungen-themen-modell-updates-design.md` (inkl. „Festlegungen aus dem Codex-Review“ und „Umsetzung“).
- Installer 0.21.11 auf `D:\lv-build\int` gebaut; Kopie samt `.notes.md` im App-Update-Ordner (`…\target\release\bundle\nsis\`).

## Was drin ist
1. **Reiter nach Thema:** Eingabe (`input`: Diktat + Mikrofon + Textverbesserungs-Kürzel/Prompts + Besprechungen + Test) · Ausgabe (`output`: Vorlesen + Töne) · KI-Modelle & Anbieter (`models`) · Allgemein (`app`) · Über. Alte gespeicherte IDs werden umgesetzt (`LEGACY_TABS` in `AppSettings.tsx`). `AGENTS.md`-Tabelle, Hilfe (de/en), Tests angepasst.
2. **Fußleiste → Anbieter-Einstellungen:** letzter Eintrag der Modellliste + Rechtsklick-Menü; `lib/openSettingsTab.ts` (Ereignis `lv-open-settings-tab`, Anker `data-settings-anchor`, `SettingsGroup anchor=`).
3. **Neue Modelle:** `managers/llm/updates.rs` (rein: Parser, Vergleich, Kandidaten, Erkennung, Ersetzen, Probelauf), Befehle `llm_check_new_models` / `llm_answer_model_updates` / `llm_set_auto_update_models` in `commands/llm.rs`, Einstellungen `llm_auto_update_models` (ask/on/off) + `llm_model_history`, `ModelUpdateGate.tsx` (20 s nach Start, dann täglich), Schalter + „neu“-Kennzeichnung in `LlmConnectionsSettings.tsx`, `cli::call_with_timeout` + Zusatzliste `set_claude_extra_models`.

## Geprüft
- Rust `managers::llm`, `settings::`, `commands::compliance`: 152 grün (davon 19 neue). tsc grün.
- Playwright: 61 grün (neue Specs `settings-themen`, `footer-provider-link`, `llm-model-updates`; angepasst `llm-connections`, `llm-usage`, `refine-setting`).
- Echter Probelauf: `claude-haiku-5-5` antwortet (≈ 0,005 $), `claude-haiku-9-9` → „issue with the selected model … may not exist“ → `absent`.
- Codex-Review der Spezifikation (gpt-6-astra; gpt-6.1-sol/gpt-6-sol lehnt die CLI 0.153.3 ab): 9 Befunde, alle eingearbeitet.

## Offen bei Patrick
- Abnahme des Installers 0.21.10 (in der App „Nach Updates suchen“).
- Push + PR (gegen `feat/protokoll-kopf-kompakt`, PR-Vorlage mit „Human Written Description“) — erst auf Ja.
- Weiterhin: Merge der Kette #76–#81, ein Release-Tag danach, Codex-CLI ≥ 0.160.1, Live-Webhook-Tests, Speicherprüfung vor Fish-Start.

## Wissenswert
- Hauptbaum hat kein vollständiges `node_modules`; Prüfen/Bauen in `D:\lv-build\int` (Dateien per Skript vom Hauptbaum kopiert, dort NICHT committen).
- Andere Sprachen kennen die neuen Schlüssel nicht (Rückfall auf en); `pnpm check:translations` ist vorbestehend rot.
- Grenzen: Claude-Probelauf höchstens 12 Kandidaten je Lauf (reihum über die Familien; mit 3 kam Haiku 5.5 erst im vierten Lauf dran), 30 s je Aufruf, 90 s gesamt, Abbruch bei Limit/Anmeldefehler; `absent` wird nach 7 Tagen neu geprüft.
