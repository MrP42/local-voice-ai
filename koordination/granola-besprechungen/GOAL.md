---
thema: granola-besprechungen
titel: Besprechungen auf Granola-Niveau - lokal, ohne Abo
state: EXECUTING
vorzustand: -
pausengrund: -
issue: 59
repo: MrP42/local-voice-ai
branch: feat/granola-besprechungen
iteration: 1
erstellt: 2026-09-29
aktualisiert: 2026-09-29T11:43
---

# Goal: Besprechungen auf Granola-Niveau - lokal, ohne Abo

## Zielzustand
Local Voice AI deckt jede Kernfunktion von Granola für Besprechungen ab (Aufnahme ohne Bot, Live-Transkript mit Sprecherzuordnung, eigene Stichpunkte → KI-Protokoll nach Vorlage, Chat/Suche über alle Besprechungen, Kalenderbezug, Export) – komplett lokal, ohne Abo und ohne laufende Kosten – und ist bei Transkriptionsqualität (Deutsch), Datenschutz und Kosten messbar gleichwertig oder besser. Erkennbar an der Feature-Matrix `koordination/granola-besprechungen/FEATURE-MATRIX.md` (jede Granola-Kernfunktion mit Status „gleichwertig/besser" und Beleg), an der Benchmark-Tabelle (WER/DER/Laufzeit) und am installierten Abnahme-Installer.

## Scope
- Analyse Granola (Features, Technik, UX, Preise) und Gap-Analyse gegen Local Voice AI → `recherche/`, `FEATURE-MATRIX.md`.
- Besprechungsfunktion in `apps/local-voice`: Audioerfassung, Transkription, Sprecher, Notizen/Protokoll, Vorlagen, Chat/Suche, Kalender, Export – soweit die Gap-Analyse sie als Kernfunktion einstuft.
- Benchmarks (Qualität, Laufzeit, Ressourcen) mit reproduzierbaren Befehlen; Tests; Doku; Installer je Abnahmestand.

## Non-Scope
- Cloud-Dienste mit laufenden Kosten, Konten/Abos, Team-/Mandantenfunktionen, gehostete Freigabelinks.
- Mobile App (iOS/Android); macOS-Parität nur so weit, dass nichts bricht (eigener Zugang, PR #32).
- Offene fremde PRs (#21, #32, #56–#58) und das Diktat-/Vorlesen-Verhalten außerhalb gemeinsam genutzter Bausteine.

## Akzeptanzkriterien
- [x] AK1 — Analyse + Plan: `recherche/granola-analyse.md`, `recherche/lokaler-stack.md`, `recherche/ist-stand.md`, `FEATURE-MATRIX.md` im Branch; Analyse-Artefakt (Link) veröffentlicht; Issue angelegt
- [ ] AK2 — Feature-Parität: `python koordination/granola-besprechungen/check_matrix.py` → Exit 0 (alle Kern-Funktionen gleichwertig/besser/vorhanden, je mit Beleg)
- [ ] AK3 — Notizblock + KI-Notizen (F07–F10): Eval über Fixture-Besprechungen mit Nutzerstichpunkten → 100 % der Nutzerstichpunkte wörtlich erhalten und als Nutzertext markiert, ≥ 95 % der KI-Aussagen mit Quellsegment belegt; Rust- und Playwright-Tests des Pfads grün
- [ ] AK4 — Transkription Deutsch: Benchmark-Befehl → WER Enddurchlauf ≤ 6 % und Live ≤ 8 % auf FLEURS-de-Stichprobe (≥ 200 Sätze); WER auf deutschem Mehrsprecher-Besprechungskorpus gemessen und in der Doku
- [ ] AK5 — Live-Latenz: Harness-Messung über ≥ 10 min Aufnahme → p95 Ende der Äußerung bis Anzeige ≤ 5 s
- [ ] AK6 — Echo: Fixture mit Lautsprecher-Echo → Ich-Transkript enthält ≤ 10 % der Gegenseite-Wörter (Baseline ohne AEC mitgemessen)
- [ ] AK7 — Sprecher: DER ≤ 15 % auf Diarisierungs-Testsatz (AMI-Stichprobe + deutsches Mehrsprecher-Fixture); Sprecher benennbar, Namen überstehen Neu-Transkription
- [ ] AK8 — Chat/Suche: Eval mit ≥ 20 Fragen über ≥ 5 Fixture-Besprechungen → ≥ 85 % richtige Antworten mit korrektem Zitat (lokales Modell); Suche < 500 ms bei 500 Besprechungen
- [ ] AK9 — Kalender + Erkennung: ICS-Fixture → Termine mit Titel/Teilnehmenden übernommen, Erinnerung; laufende Meeting-App (Mikrofonnutzung) → Hinweis „Aufnahme starten?"; Tests grün
- [ ] AK10 — Nachbereitung/Export: Follow-up-Mail-Entwurf, formatierte Zwischenablage, PDF/SRT/JSON-Export; Tests grün
- [ ] AK11 — Abnahme: Installer gebaut und installiert, Screenshots der Kernabläufe (Aufnahme mit Notizen → KI-Notizen → Chat) in `koordination/granola-besprechungen/abnahme/`

## Quality Gates
- [ ] QG1 — Rust komplett grün: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib`
- [ ] QG2 — Frontend komplett grün: `npx tsc --noEmit` + `pnpm test:playwright`; eigene Dateien eslint/prettier-sauber
- [ ] QG3 — Performance: 60-min-Besprechung auf RTX 4090 → Enddurchlauf + KI-Notizen ≤ 3 min nach Stopp; CPU-only-Pfad funktioniert (Messung dokumentiert)
- [ ] QG4 — Systemschutz: alle neuen Modelle/Prozesse hinter RAM-Start-Gate/Deckel; Test mit knappem RAM → sauberer Abbruch statt Einfrieren
- [ ] QG5 — Datenschutz: Besprechungspfad ohne Netzverkehr (außer Modell-Download und bewusst gewähltem externem LLM), Nachweis per Offline-Lauf
- [ ] QG6 — Lizenzen: jedes neue Modell/Crate mit Lizenz in den Third-Party-Notices, keine Nicht-kommerziell-Lizenz
- [ ] QG7 — i18n: alle neuen Texte in de + en, echte Umlaute
- [ ] QG8 — Doku + Handoff aktualisiert, PR offen gegen `main`
- [ ] QG9 — Budget ≤ 8 MTok (Meldung bei 4,0 / 6,4 MTok, harter Stopp 12 MTok)

## Constraints
- Lokal und kostenfrei im Betrieb: nur Open-Source-/frei nutzbare Modelle und Bibliotheken, Lizenz je Baustein geprüft (keine Nicht-kommerziell-Klauseln ohne Ansage).
- Systemschutz: kein Einfrieren von Windows (RAM/CPU); neue Kindprozesse über `process_guard.rs`, RAM-Start-Gate, Speicherwächter.
- Repo-Regeln `AGENTS.md`: Branch + PR, nie Direkt-Push auf `main`, keine Formatierläufe über fremde Dateien, vorbestehendes Rot (prettier/clippy/translations) nicht mitreparieren.
- Einstellungen nur in vorhandene Gruppen (kein neuer Menüpunkt/Reiter für Einstellungen); Besprechungen bleiben die Tätigkeit „Besprechungen" in der Seitenleiste.
- Jedes Feature: eigener Zweig/Prototyp → härten → Einbau → Gesamttest (Memory „Feature erst als Prototyp reifen"); Abnahme nur per Installer, Patch-Version +1 je Abnahmestand; Release-Tag erst gebündelt am Ende.

## Architekturprinzipien
- An vorhandene Manager/Commands/Settings/bindings anschließen, keine Parallelstrukturen.
- Aufnahme ist verlustfrei und absturzsicher (erst schreiben, dann verarbeiten); Transkription und KI laufen nachgelagert und sind wiederholbar.
- Zwei Qualitätsstufen: schnelles Live-Transkript während der Besprechung, hochwertiger Enddurchlauf danach.
- Nutzertext und KI-Text bleiben unterscheidbar; das LLM erfindet nichts, was nicht im Transkript steht (Belege/Zeitstempel).
- GPU/RAM-Koordination: STT, Diarisierung und LLM teilen sich Ressourcen geordnet (laden/entladen), nie gleichzeitig über Budget.

## Dependencies
- Gebündelter `llama-server` (Sprachmodelle-Vollausbau) für lokale LLM-Aufgaben.
- Vorhandene STT-Engines der Diktatfunktion; Modell-Download/Katalog (`catalog.json`).

## Risiken / Owner-Entscheidungen
- R1 Budget ~8 MTok (Spanne 6–10) über mehrere Sessions — Vorschlag: so fahren, Meldung je Meilenstein. Owner: Patrick.
- R2 Systemton-Aufnahme standardmäßig an (Einwilligungsdialog bleibt) — Vorschlag: ja.
- R3 Kalender: zuerst ICS-URL (kein Konto), danach Microsoft Graph mit eigener Client-ID (PKCE) — Vorschlag: so; Google erst bei Bedarf.
- R4 VRAM: großes LLM (~21 GB) kollidiert mit STT/Diarisierung/Fish → GPU-Koordinator, LLM erst nach Enddurchlauf laden.
- R5 Modellnamen/Zahlen aus der Recherche teils unbelegt (Nemotron-3-Diarization, Qwen3.x-27B, `sonora`) → jeder Baustein erst im Spike verifiziert (Download, Lizenz, Messung), dann Einbau.
- R6 Diarisierung auf deutschen Besprechungen mit > 4 Sprechern und Überlappung unsicher → Rückfall Sortformer; Sprecher manuell korrigierbar.
- R7 Kein öffentlicher deutscher Besprechungs-Benchmark → Eigenkorpus (TTS-Dialoge mit mehreren Stimmen + Echo/Überlappung simuliert), optional echte Aufnahmen von Patrick mit Referenztext.
- R8 Sprechernamen aus Teams/Zoom-Oberfläche (wie Granola, UI Automation) — Komfort, nach M3.
- E1 Aufbewahrung: Default `AfterMinutes` löscht Audio nach dem Protokoll; KI-Notizen lösen die Löschung nicht aus (Entwurf M1). Vorschlag: Default für neue Nutzer auf 30 Tage, damit der Audio-Sprung (F08 „besser") dauerhaft wirkt — bis zur Entscheidung unverändert.
- E2 KI-Notizen nach Stopp automatisch (Default an, nur mit konfiguriertem Anbieter) — Vorschlag: ja.
- E3 Nutzertext bleibt bei „Anweisung anwenden" unverändert — Vorschlag: ja.
- E4 Eigener Block-Editor statt TipTap (keine neue Abhängigkeit) — Vorschlag: ja.
- E5 GPU-STT im Windows-Release (P2f): heute rechnet STT nur auf der CPU (`release-windows.yml` ohne `gpu-vulkan`). Vorschlag: Vulkan-Feature im Release aktivieren (CI hat das SDK schon), lokal LunarG-Vulkan-SDK installieren (`winget install KhronosGroup.VulkanSDK`, Admin). Ohne GPU gilt: Live-Transkript (Parakeet, CPU) ist zugleich Endtranskript.
- R9 Lokaler Kontext fest 8192 Token → Map-Reduce als Normalfall; misst P1e unter 95 % Belegquote, folgt Paket P1g (Kontext je VRAM größer, `ensure_local`).

## Meilensteine
| M | Ergebnis (anfassbar) | Status |
|---|---|---|
| M0 | Analyse-Artefakt (Link) + Feature-Matrix + Paketplan, Issue | offen |
| M1 | Notizblock während der Aufnahme + KI-Notizen nach Vorlage mit Nutzer-/KI-Unterscheidung und Quellsprung (Installer + Screenshot) | offen |
| M2 | Audio/STT: AEC, VAD, Live ≤ 5 s, Enddurchlauf, Ausfallwächter, Benchmark-Tabelle | offen |
| M3 | Sprecher: Diarisierung, Benennen, Wiedererkennen (DER-Tabelle) | offen |
| M4 | Chat + Suche + Ordner über alle Besprechungen (Eval-Tabelle) | offen |
| M5 | Kalender, Meeting-Erkennung, Personen, Brief | offen |
| M6 | Follow-up-Mail, Export, lokaler MCP-Server | offen |
| M7 | Gesamttest, Performance, Offline-Nachweis, Installer, Doku, PR | offen |

## Evidence
- 2026-09-29T11:13 AK1 erfüllt — Recherche-Dateien + FEATURE-MATRIX.md in 45f3a3b, Artefakt https://claude.ai/artifact/JL7yCpwFDTcrvAm9Xj2Dwo (Kopie 91217fa), Issue #59
- 2026-09-29T11:41 QG1 widerrufen — frühere Belege gelten nicht mehr

## Blocker
- B1 [offen] [P2f] 2026-09-29T11:28 Ursache: Lokaler Vulkan-Build braucht das LunarG-SDK (Installation mit Admin-Rechten) und aendert den Release-Build (E5) · Owner: Patrick · entsperrt, wenn: Patrick gibt E5 frei (SDK installiert oder CUDA-Weg gewaehlt) oder lehnt ab (dann P2f abgebrochen, CPU-Pfad) · nächste Prüfung: beim nächsten Sessionstart

## Entscheidungen
- 2026-09-29 M1/M2-Berührpunkt B4: Auto-KI-Notizen starten auf `MeetingEvent::TranscriptFinal` (P2a führt es ein und sendet es am Ende von `stop()`, P1f hängt sich daran, P2d verschiebt das Senden hinter den Enddurchlauf). Enddurchlauf nutzt `segment_epoch` aus P1a; M2 liefert `remap_sources()`; M2 fügt keine meetings.db-Migration hinzu.
- 2026-09-29 Silero v6 verschoben (vad-rs lädt v6 nicht); M2 nutzt v4.
- 2026-09-29 Worker-Routing (Patricks Auftrag, zugleich Freigabe für Subagents nach der M9-Regel): Orchestrator Opus 5.5; Coding `lv-coder` (Sonnet 5.5 high), riskante Pakete `lv-coder-xhigh` (Sonnet 5.5 xhigh), Architektur/Spike/Review `lv-architect` (Opus 5.5 high); kleine Re-Reviews Haiku. Definitionen in `.claude/agents/`.
- 2026-09-29 Branch basiert auf `chore/0.20.3-abnahme` (PR #58 = #56 + #57), damit Abnahme-Installer die installierten Fixes enthalten.
- 2026-09-29 Reihenfolge: M1 (Notizblock/KI-Notizen = Kern der Granola-Identität) und M2 (Audio/STT) parallel auf disjunkten Dateien; M3 nach M2; M4 nach M1.

## Nächste empfohlene Aktion
Lieferungen validieren und seriell in feat/granola-besprechungen mergen (Konflikte lib.rs/bindings.ts/recorder.rs); dann W3: P1d, P1e, P1f, P2c2, P2b2, P2e; M3-Spike wenn Builds ruhen

## Verlauf
- 2026-09-29T10:40 DISCOVERY — Goal State angelegt
- 2026-09-29T11:02 DISCOVERY (Runde 0) — Metadaten: issue=59
- 2026-09-29T11:02 READY (Runde 0) — Goal definiert, Issue #59, Discovery abgeschlossen (3 Recherchen)
- 2026-09-29T11:04 PLANNING (Runde 1) — Pakete P0-P2 geplant
- 2026-09-29T11:04 EXECUTING (Runde 1) — P0 (Artefakt, Sonnet), P1 (Entwurf M1, lv-architect), P2 (Spike+Entwurf M2, lv-architect) laufen
- 2026-09-29T11:18 EXECUTING (Runde 1) — P0+P1 abgenommen (Artefakt JL7yCpwFDTcrvAm9Xj2Dwo, Entwurf M1 4b12c80); P1a (lv-coder-xhigh, wt-m1) und P2 (Spike M2) laufen; Budget ~1,4 MTok
- 2026-09-29T11:23 EXECUTING (Runde 1) — Laufwerk C: war bei 34 GB frei (99 %); Build-Caches (target) von 4 alten, in main gemergten Worktrees geloescht -> 70 GB frei. Haupt-target 63 GB. Vor Installer-Builds freien Platz pruefen (>= 20 GB).
- 2026-09-29T11:28 EXECUTING (Runde 1) — B1 blockiert Paket P2f: Lokaler Vulkan-Build braucht das LunarG-SDK (Installation mit Admin-Rechten) und aendert den Release-Build (E5) – übrige Pakete laufen weiter
- 2026-09-29T11:29 EXECUTING (Runde 1) — P1a (wt-m1), P2c1 (wt-m2), P2b1 (wt-m2b) laufen parallel; P2f blockiert (E5); Budget ~1,75 MTok
- 2026-09-29T11:43 EXECUTING (Runde 1) — P1a abgenommen (13fa4d3, 732 Tests). Laufend: P1b (wt-m1), P1c (wt-m1c), P2a (wt-m2a), P2c1 (wt-m2), P2b1 (wt-m2b), P4-Entwurf. CARGO_BUILD_JOBS=8 je Worker. Budget ~2,0 MTok

