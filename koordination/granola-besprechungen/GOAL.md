---
thema: granola-besprechungen
titel: Besprechungen auf Granola-Niveau - lokal, ohne Abo
state: BLOCKED
vorzustand: EXECUTING
pausengrund: -
issue: 59
repo: MrP42/local-voice-ai
branch: feat/granola-besprechungen
iteration: 3
erstellt: 2026-09-29
aktualisiert: 2026-09-30T18:20
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
- [x] AK2 — Feature-Parität: `python koordination/granola-besprechungen/check_matrix.py` → Exit 0 (alle Kern-Funktionen gleichwertig/besser/vorhanden, je mit Beleg)
- [x] AK3 — Notizblock + KI-Notizen (F07–F10): Eval über Fixture-Besprechungen mit Nutzerstichpunkten → 100 % der Nutzerstichpunkte wörtlich erhalten und als Nutzertext markiert, ≥ 95 % der KI-Aussagen mit Quellsegment belegt; Rust- und Playwright-Tests des Pfads grün
- [x] AK4 — Transkription Deutsch: Benchmark-Befehl → WER Enddurchlauf ≤ 6 % und Live ≤ 8 % auf FLEURS-de-Stichprobe (≥ 200 Sätze); WER auf deutschem Mehrsprecher-Besprechungskorpus gemessen und in der Doku
- [x] AK5 — Live-Latenz: Harness-Messung über ≥ 10 min Aufnahme → p95 Ende der Äußerung bis Anzeige ≤ 5 s
- [x] AK6 — Echo: Fixture mit Lautsprecher-Echo → Ich-Transkript enthält ≤ 10 % der Gegenseite-Wörter (Baseline ohne AEC mitgemessen)
- [x] AK7 — Sprecher: DER ≤ 15 % auf Diarisierungs-Testsatz (AMI-Stichprobe + deutsches Mehrsprecher-Fixture); Sprecher benennbar, Namen überstehen Neu-Transkription
- [x] AK8 — Chat/Suche: Eval mit ≥ 20 Fragen über ≥ 5 Fixture-Besprechungen → ≥ 85 % richtige Antworten mit korrektem Zitat (lokales Modell); Suche < 500 ms bei 500 Besprechungen
- [x] AK9 — Kalender + Erkennung: ICS-Fixture → Termine mit Titel/Teilnehmenden übernommen, Erinnerung; laufende Meeting-App (Mikrofonnutzung) → Hinweis „Aufnahme starten?"; Tests grün
- [x] AK10 — Nachbereitung/Export: Follow-up-Mail-Entwurf, formatierte Zwischenablage, PDF/SRT/JSON-Export; Tests grün
- [ ] AK11 — Abnahme: Installer gebaut und installiert, Screenshots der Kernabläufe (Aufnahme mit Notizen → KI-Notizen → Chat) in `koordination/granola-besprechungen/abnahme/`

## Quality Gates
- [x] QG1 — Rust komplett grün: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib`
- [x] QG2 — Frontend komplett grün: `npx tsc --noEmit` + `pnpm test:playwright`; eigene Dateien eslint/prettier-sauber
- [x] QG3 — Performance: 60-min-Besprechung auf RTX 4090 → Enddurchlauf + KI-Notizen ≤ 3 min nach Stopp; CPU-only-Pfad funktioniert (Messung dokumentiert)
- [x] QG4 — Systemschutz: alle neuen Modelle/Prozesse hinter RAM-Start-Gate/Deckel; Test mit knappem RAM → sauberer Abbruch statt Einfrieren
- [x] QG5 — Datenschutz: Besprechungspfad ohne Netzverkehr (außer Modell-Download und bewusst gewähltem externem LLM), Nachweis per Offline-Lauf
- [x] QG6 — Lizenzen: jedes neue Modell/Crate mit Lizenz in den Third-Party-Notices, keine Nicht-kommerziell-Lizenz
- [x] QG7 — i18n: alle neuen Texte in de + en, echte Umlaute
- [x] QG8 — Doku + Handoff aktualisiert, PR offen gegen `main`
- [ ] QG9 — Budget ≤ 19 MTok (30.09. von 16 auf 19 angehoben durch Patrick für Runde 3 + Protokoll-Vorlagen; 29.09. abends von 12 auf 16; Meldung bei 6,0 / 9,6 MTok, harter Stopp 18 MTok)

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
- E6 Semantische Suche lädt 635 MB (BGE-M3) per Knopf, nicht automatisch — Vorschlag: ja.
- E7 Live-Chat mit lokalem LLM nur mit GPU-Backend (Schutz der Live-Latenz) — Vorschlag: ja.
- E8 Chats werden gespeichert (Threads je Scope) — Vorschlag: ja.
- E9 Ordner flach, eine Besprechung in mehreren Ordnern (n:m) — Vorschlag: ja.
- E10–E17 (Entwurf M5/M6 §11): Erkennung Standard „Meeting-Apps“ nur als Hinweis; Erinnerung 1 min nur bei ≥ 2 Teilnehmenden oder Join-URL; Geheimnisse DPAPI-verschlüsselt, nie im Sync; MCP standardmäßig aus mit Warnung; Graph-Client-ID registriert Patrick in Entra (P5f danach); PDF über WebView2; Briefs nur per Knopf; kein Massenexport in M6 — Vorschlag jeweils: ja.
- E18–E23 (Entwurf M3 §7): Sortformer 4spk jetzt, Nemotron später; transcribe-cpp 0.2.4 app-weit mit WER-Regressionstor; Diarisierung Standard an, Wiedererkennen Standard aus mit Einwilligung je Person; AK7 auf AMI knapp → Bericht abwarten, nicht nachtunen; Lizenzhinweise NVIDIA Open Model/OpenMDW in „Über“; E23 echte deutsche Testaufnahme 3–5 Personen von Patrick (sonst deutscher AK7-Teil synthetisch) — Vorschlag jeweils: ja.
- R9 Lokaler Kontext fest 8192 Token → Map-Reduce als Normalfall; misst P1e unter 95 % Belegquote, folgt Paket P1g (Kontext je VRAM größer, `ensure_local`).

## Meilensteine
| M | Ergebnis (anfassbar) | Status |
|---|---|---|
| M0 | Analyse-Artefakt (Link) + Feature-Matrix + Paketplan, Issue | erfüllt (Artefakt JL7yCpwFDTcrvAm9Xj2Dwo, Matrix, Issue #59) |
| M1 | Notizblock während der Aufnahme + KI-Notizen nach Vorlage mit Nutzer-/KI-Unterscheidung und Quellsprung (Installer + Screenshot) | erfüllt (P1a–P1i; Eval 30/30, belegt; Screenshots abnahme/) |
| M2 | Audio/STT: AEC, VAD, Live ≤ 5 s, Enddurchlauf, Ausfallwächter, Benchmark-Tabelle | erfüllt (P2a–P2g; live p95 2,0 s, WER 4,17 %/5,51 %, Echo 0 %, bench.md) |
| M3 | Sprecher: Diarisierung, Benennen, Wiedererkennen (DER-Tabelle) | erfüllt ohne Wiedererkennen (P3a–P3c; DER de 0,94 %, AMI 15,19 %); P3d Folge-Goal |
| M4 | Chat + Suche + Ordner über alle Besprechungen (Eval-Tabelle) | erfüllt (P4a–P4g; Chat-Eval 24/24) |
| M5 | Kalender, Meeting-Erkennung, Personen, Brief | erfüllt (P5a–P5f; ICS + Graph, Erkennung, Personen, Brief) |
| M6 | Follow-up-Mail, Export, lokaler MCP-Server | erfüllt (P6a–P6e; Export 8 Formate, Follow-up, MCP) |
| M7 | Gesamttest, Performance, Offline-Nachweis, Installer, Doku, PR | Abnahme läuft (Installer 0.20.8 installiert 30.09.; Rückmeldung Patrick -> Runde 3: P8a, P6f, P3e) |

## Evidence
- 2026-09-29T11:13 AK1 erfüllt — Recherche-Dateien + FEATURE-MATRIX.md in 45f3a3b, Artefakt https://claude.ai/artifact/JL7yCpwFDTcrvAm9Xj2Dwo (Kopie 91217fa), Issue #59
- 2026-09-29T11:41 QG1 widerrufen — frühere Belege gelten nicht mehr
- 2026-09-29T16:53 QG2 widerrufen — frühere Belege gelten nicht mehr
- 2026-09-29T17:21 AK3 erfüllt — --eval-notes (Gemma 4 E4B, CUDA): user_preserved 1.0 (30/30), ai_sourced 1.0 (21/21), abnahme/p1e-eval-notes.json, Commit 2ec9c328; Rust 1012 gruen, Playwright meeting-notes 34 gruen
- 2026-09-29T17:36 AK6 erfüllt — --simulate-meeting m2_echo_mic/render: ich_far_word_leak mit AEC 0,00, Baseline --no-aec 0,565; Commit ea2a398f
- 2026-09-29T18:57 AK5 erfüllt — m2-bench.ps1 -Full: 12,8 min Echtzeit, 128 Segmente, Latenz p95 1385 ms (p50 960, max 4604), docs/m2-evidence/bench.md, Commit d0f3391e
- 2026-09-29T18:57 AK4 erfüllt — docs/m2-evidence/bench.md: FLEURS-de 240 Saetze Enddurchlauf Qwen3-ASR 1.7B 4,17 % (nach Upgrade 0.2.4 +0,21 Pp), Whisper large-v3 CUDA 4,65 %; Live Parakeet ONNX 7,86 %; Mehrsprecher-Korpus Live 6,86 %; Commits 44c4bde, d0f3391e
- 2026-09-29T19:33 AK8 erfüllt — --eval-chat 24 Fragen/5 Besprechungen: Gemma 4 E4B accuracy 1,000 in 3 von 3 Laeufen (abnahme/p4g-eval-chat-1..3.json), 12B 1,000; Suche ui p95 180 ms (Release, P4a) bei 500 Besprechungen; Commit a4207200
- 2026-09-29T21:51 AK7 erfüllt — P3a/P3b/P3c: AMI-Pruefteil 15,19 % (Patrick wertet als erfuellt, B7), deutsch 0,94 %, Benennen + Namen ueberstehen Neu-Transkription (a97a1008)
- 2026-09-29T21:55 AK9 erfüllt — P5a/P5b 892c3291 + P5c 34c948e2: ICS-Fixture-Tests, reminder-Tests, --detect-mic; cargo 1552 passed, Playwright 205 passed
- 2026-09-29T22:18 AK10 erfüllt — P6a/P6b/P6c/P6d 95e21c66: Follow-up-Mail, formatiert kopieren, PDF/SRT/JSON; Playwright 214 passed, cargo 1552 passed
- 2026-09-29T22:35 QG6 erfüllt — P7c 31ccc1f7: ATTRIBUTION.md + Info-Seite, alle neuen Crates/Modelle frei (BSD/MIT/Apache/CC-BY, Sortformer NVIDIA Open Model); vorbestehende Funde B10
- 2026-09-29T22:35 QG7 erfüllt — P7c 31ccc1f7: check_i18n_meetings.py Exit 0 (640 Schluessel de/en, keine Umlaut-Ersatzschreibung)
- 2026-09-30T00:30 QG4 erfüllt — P7b ea876e60: abnahme/p7b-qg4.md, knapper RAM -> sauberer Abbruch je Schritt (LowRam/memory_low/pdf_low_memory), keine Restprozesse
- 2026-09-30T00:30 QG5 erfüllt — P7b ea876e60: abnahme/p7b-qg5.md, ganze Kette 0 Nicht-Loopback-Verbindungen (94 Sockets 127.0.0.1), Proxy 0 Anfragen
- 2026-09-30T00:30 AK2 erfüllt — check_matrix.py Exit 0: 22/22 Kern-Funktionen gleichwertig/besser/vorhanden mit Beleg (Stand 2ea33a91)
- 2026-09-30T14:28 QG3 erfüllt — P7b+P1i 25e32343: 60 min RTX 4090 Gemma 4 E4B Stopp->KI-Notizen 156,6 s (3 Laeufe 156-158 s); CPU-only funktioniert (abnahme/p7b-qg3.md)
- 2026-09-30T14:28 QG1 erfüllt — cargo test --lib 1748 passed 0 failed (25e32343)
- 2026-09-30T14:28 QG2 erfüllt — tsc 0, Playwright 251 passed 0 failed (25e32343); eigene Dateien eslint/prettier je Paket geprueft
- 2026-09-30T15:50 QG8 erfüllt — docs/BESPRECHUNGEN.md, KNOWN-LIMITATIONS, STATUS (P7c); PR #63 offen (inkl. #62); Handoff 2026-09-30

## Blocker
- B1 [gelöst] [P2f] 2026-09-29T11:28 Ursache: Lokaler Vulkan-Build braucht das LunarG-SDK (Installation mit Admin-Rechten) und aendert den Release-Build (E5) · Owner: Patrick · entsperrt, wenn: Patrick gibt E5 frei (SDK installiert oder CUDA-Weg gewaehlt) oder lehnt ab (dann P2f abgebrochen, CPU-Pfad) · nächste Prüfung: beim nächsten Sessionstart · gelöst 2026-09-29T12:10: Patrick 29.09.: Vulkan ja, Claude installiert das SDK (winget KhronosGroup.VulkanSDK)
- B2 [gelöst] 2026-09-29T20:32 Ursache: Freigegebener Budgetrahmen 12 MTok erreicht (Ist ~12,8 MTok); Hochrechnung bis COMPLETE 15-16 MTok · Owner: Patrick · entsperrt, wenn: Patrick hebt den Rahmen an (Vorschlag 16 MTok) oder kuerzt den Umfang (Komfortpakete P3d/P5d/P5e/P5f/P6e in Folge-Goal); dazu offen: B7, E14, E23 · nächste Prüfung: beim nächsten Sessionstart · gelöst 2026-09-29T21:51: Patrick 29.09. abends: Rahmen 16 MTok, B7 a, E14 als Einstellung spaeter, E23 spaeter
- B3 [gelöst] 2026-09-30T15:50 Ursache: AK11: Abnahme per Installer 0.20.8 (inkl. #62) braucht Installation und Pruefung durch Patrick; Installer liegt unter apps/local-voice/src-tauri/target/release/bundle/nsis/ · Owner: Patrick · entsperrt, wenn: Patrick hat 0.20.8 installiert und Kernablauf geprueft (Aufnahme mit Notizen -> KI-Notizen -> Chat), Screenshots/Rueckmeldung liegen vor · nächste Prüfung: beim nächsten Sessionstart · gelöst 2026-09-30T16:33: Patrick hat 0.20.8 installiert und durchgeklickt (30.09.); Rueckmeldung -> P8a, P6f, P3e
- B4 [offen] 2026-09-30T18:20 Ursache: AK11: Abnahme per Installer 0.20.9 (Runde 3) durch Patrick · Owner: Patrick · entsperrt, wenn: Patrick hat 0.20.9 installiert und Runde 3 geprueft (Fortschritt/Pause/Stopp, Protokoll mit Vorlage/Automatik, Follow-up) · nächste Prüfung: beim nächsten Sessionstart

## Entscheidungen
- 2026-09-29 Patrick: voller Umfang M1–M7 in diesem Goal, Budgetrahmen ~12 MTok (Hochrechnung nach Ist 3,0 MTok).
- 2026-09-30 Patrick: Protokoll-Reiter bekommt Vorlagenwahl inkl. „Automatisch (nach Inhalt)“, Protokoll über die P1i-Blocklogik (B12) -> P1k, jetzt mit Runde 3; Budgetrahmen 19 MTok.
- 2026-09-30 Patrick nach Abnahme 0.20.8: Fortschrittsanzeige (Balken, %, Laufzeit, Restdauer) für die Verarbeitung, Autoscroll im Transkript (abwählbar), Pause und Stopp mit Nachfrage -> P8a; Mehrsprecher-Test mit Emilia-Sofie-Mara-MP3 -> P3e; Vorlage automatisch nach Inhalt -> Folge-Goal.
- 2026-09-29 abends Patrick: Budgetrahmen 16 MTok; B7 → AK7 fuer AMI als erfuellt gewertet (Nemotron als Folge-Goal); E14 Graph-Client-ID spaeter vom Nutzer in den Einstellungen eintragbar (Feld in vorhandener Kalender-Gruppe, kein fester Wert im Code); E23 echte Testaufnahme spaeter (deutscher Teil bleibt synthetisch).
- 2026-09-29 Patrick zu E5: GPU-STT per Vulkan aktivieren; Claude installiert das LunarG-Vulkan-SDK (winget) und alles Nötige.
- 2026-09-29 M1/M2-Berührpunkt B4: Auto-KI-Notizen starten auf `MeetingEvent::TranscriptFinal` (P2a führt es ein und sendet es am Ende von `stop()`, P1f hängt sich daran, P2d verschiebt das Senden hinter den Enddurchlauf). Enddurchlauf nutzt `segment_epoch` aus P1a; M2 liefert `remap_sources()`; M2 fügt keine meetings.db-Migration hinzu.
- 2026-09-29 Silero v6 verschoben (vad-rs lädt v6 nicht); M2 nutzt v4.
- 2026-09-29 Worker-Routing (Patricks Auftrag, zugleich Freigabe für Subagents nach der M9-Regel): Orchestrator Opus 5.5; Coding `lv-coder` (Sonnet 5.5 high), riskante Pakete `lv-coder-xhigh` (Sonnet 5.5 xhigh), Architektur/Spike/Review `lv-architect` (Opus 5.5 high); kleine Re-Reviews Haiku. Definitionen in `.claude/agents/`.
- 2026-09-29 Branch basiert auf `chore/0.20.3-abnahme` (PR #58 = #56 + #57), damit Abnahme-Installer die installierten Fixes enthalten.
- 2026-09-29 Reihenfolge: M1 (Notizblock/KI-Notizen = Kern der Granola-Identität) und M2 (Audio/STT) parallel auf disjunkten Dateien; M3 nach M2; M4 nach M1.

## Nächste empfohlene Aktion
B4 auflösen (Patrick): Patrick hat 0.20.9 installiert und Runde 3 geprueft (Fortschritt/Pause/Stopp, Protokoll mit Vorlage/Automatik, Follow-up). Dann `goal.py resolve-blocker --id B4 --beleg …` und `goal.py set --state PLANNING`.

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
- 2026-09-29T12:05 EXECUTING (Runde 1) — BUDGET: Ist ~3,0 MTok (Subagents 2,7 + Orchestrator ~0,35). Real je Coder-Paket 250-310 kTok, je Entwurf 170-240 kTok. Hochrechnung voller Umfang (M1-M7, ~33 Pakete offen) ~12 MTok = +50 % ueber Schaetzung 8 -> Rueckfrage an Patrick (Regel: anhalten und fragen). Abgenommen: P0, P1, P2, P1a, P1c, P2c1; laufend P1b, P2a, P2b1; P4 geliefert.
- 2026-09-29T12:10 EXECUTING (Runde 1) — B1 gelöst: Patrick 29.09.: Vulkan ja, Claude installiert das SDK (winget KhronosGroup.VulkanSDK)
- 2026-09-29T12:12 EXECUTING (Runde 1) — Vulkan-SDK installiert (C:\VulkanSDK\1.4.357.0, VULKAN_SDK Machine). Laufend: P1b, P2a, P2b1, P4a, P5. P2f startet, sobald ein Build frei ist (CPU 74 %, RAM frei 23,7 GB).
- 2026-09-29T12:50 PAUSED (Runde 1) — Nutzungslimit naht (Patrick). Abgenommen+gemergt: P0,P1,P2,P4,P5 (Entwuerfe), P1a,P1b,P1c,P2a,P2c1. Beim Pausieren noch laufend/uncommittet in Worktrees: P1d (wt-m1), P1f (wt-m1c), P2b1 (wt-m2b), P4a (wt-m4), P2f (wt-m2). Budget ~4,3 MTok. [Pause: limit]
- 2026-09-29T16:29 EXECUTING (Runde 1) — wiederaufgenommen
- 2026-09-29T19:39 EXECUTING (Runde 1) — Stand 29.09. abends: AK1,3,4,5,6,8 erfuellt; Matrix 16/22 (inkl. F13). Gemergt: P1a-f, P2a-f, P2b1/2, P3a/b, P4a-g, P5a, P6a. Offen AK2,7(B7 Owner),9,10,11. Budget ~11 MTok, Hochrechnung 15-16. Laufend P3c (wt-m1), P5b (wt-m1c), P5c (wt-m2a), P6c (wt-m2b).
- 2026-09-29T20:32 BLOCKED (Runde 1) — BLOCKIERT B2 (global, keine unabhängige Arbeit mehr): Freigegebener Budgetrahmen 12 MTok erreicht (Ist ~12,8 MTok); Hochrechnung bis COMPLETE 15-16 MTok
- 2026-09-29T21:51 BLOCKED (Runde 1) — B2 gelöst: Patrick 29.09. abends: Rahmen 16 MTok, B7 a, E14 als Einstellung spaeter, E23 spaeter
- 2026-09-29T21:51 PLANNING (Runde 2) — Blocker B2 aufgeloest (Rahmen 16 MTok), AK7 erfuellt
- 2026-09-29T21:56 EXECUTING (Runde 2) — P7a (wt-m1), P1g (wt-m2a), P2g (wt-m2b) laufen; Budget ~12,9 von 16 MTok (80 %-Marke erreicht)
- 2026-09-29T23:12 EXECUTING (Runde 2) — P5f (wt-m1c), P7b (wt-m2b) laufen; gemergt P7a,P6d,P7c,P6e,P5d/e,P2g,P1g; Befunde alle erledigt; Budget ~15 MTok
- 2026-09-30T15:50 BLOCKED (Runde 2) — BLOCKIERT B3 (global, keine unabhängige Arbeit mehr): AK11: Abnahme per Installer 0.20.8 (inkl. #62) braucht Installation und Pruefung durch Patrick; Installer liegt unter apps/local-voice/src-tauri/target/release/bundle/nsis/
- 2026-09-30T16:33 BLOCKED (Runde 2) — B3 gelöst: Patrick hat 0.20.8 installiert und durchgeklickt (30.09.); Rueckmeldung -> P8a, P6f, P3e
- 2026-09-30T16:33 PLANNING (Runde 3) — Runde 3 nach Abnahme-Rueckmeldung: P8a, P6f, P3e
- 2026-09-30T16:34 EXECUTING (Runde 3) — P8a (wt-m1), P6f (wt-m1c), P3e (wt-m2b) laufen; Budget ~16,8 MTok
- 2026-09-30T18:20 BLOCKED (Runde 3) — BLOCKIERT B4 (global, keine unabhängige Arbeit mehr): AK11: Abnahme per Installer 0.20.9 (Runde 3) durch Patrick

