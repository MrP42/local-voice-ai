# Befunde des Planers — Modulare TTS-Anbieter, Phonon/Ollama-Speech und reproduzierbarer deutscher TTS-Benchmark

(Je Befund: Überschrift `## B<n> — <Paket>: <Titel>`, Zeilen Beobachtung / Beleg / Konsequenz und
eine Zeile `- Status: offen` bzw. `- Status: erledigt (<Paket/Commit>)`. Offene Befunde verhindern COMPLETE.)

## B2 — P1: Piper-Abbruch beendet den Kindprozess erst nach dessen Frist
- Beobachtung: `run_piper_blocking` (piper.rs) laeuft auf einem Blocking-Thread; Abbruch beendet das Warten sofort, der Prozess endet erst nach Auftragsfrist + 2 s.
- Beleg: P1-Report, Risiko 1.
- Konsequenz: Als bekannte Grenze in docs/tts dokumentieren (Piper-Interna sind Non-Scope); kein Ressourcenleck, da Frist begrenzt.
- Status: erledigt (als bekannte Grenze dokumentiert: docs/tts/installation-troubleshooting.md, CHANGELOG)

## B3 — P1: Lizenzwiderspruch Piper thorsten-high
- Beobachtung: provider-research.md nennt den thorsten-Datensatz CC0, der App-Katalog fuehrt thorsten-high als non_commercial (Ableitung von lessac).
- Beleg: P1-Report, Risiko 4; models.rs-Test.
- Konsequenz: Huelle bleibt konservativ (Katalogwert, verified=false); Widerspruch in docs/tts mit Quellen benennen, Owner klaert.
- Status: offen

## B4 — vorbestehend: commands::meeting_notes::tests::import_rejects_bad_files_with_stable_codes rot
- Beobachtung: assert 9 == 8 (mitgelieferte Besprechungsvorlagen), meeting_notes.rs:335.
- Beleg: `cargo test --lib commands::` auf feat/tts-provider; Datei vom Goal unberuehrt (`git diff --name-only` ohne meeting_notes).
- Konsequenz: nicht Teil dieses Goals (AGENTS.md: vorbestehendes Rot nicht mitreparieren); im PR benennen.
- Status: erledigt (als vorbestehend dokumentiert, kein Goal-Befund)

## B5 — P4: Attrappe der Playwright-Spec zeigt falsche Anbieterdaten
- Beobachtung: Screenshot tts-anbieter-gruppe.png zeigt Phonon mit CUDA, echtem Streaming, Version 0.3.1 und Piper als „verifiziert am 2026-10-26“; die echten Deskriptoren melden Phonon CPU/satzweise und Piper verified=false.
- Beleg: D:/lv-build/tmp/p4-screens/tts-anbieter-gruppe.png vs. phonon.rs/builtin.rs.
- Konsequenz: Screenshot nicht als Beleg verwenden; Paket P4n leitet die Attrappe aus echter CLI-Ausgabe ab.
- Status: erledigt (P4n 328d747b)

## B6 — P2: Phonon-Lizenzhinweis mit leerem Modellnamen
- Beobachtung: Ohne eingetragenes Modell lautet die Notiz "die Lizenz der Gewichte von '' ist nicht belegt".
- Beleg: docs/tts/img/tts-anbieter-gruppe.png, phonon.rs Lizenznotiz.
- Konsequenz: Text bei leerem Modell auf "kein Modell eingetragen" umstellen (niedrig, mit Review-Nacharbeit).
- Status: erledigt (P7n c21d9fab)

## B7 — P7: Widerruf wirkt nicht auf laufende Auftraege; Pruefung nicht an die benutzte Instanz gebunden (Review 1, 2, 8)
- Beobachtung: Freigabe-Befehle (commands/tts_providers.rs:296-342) rufen kein refresh_from_settings; ProviderEngine.guard (pipeline.rs:177) prueft einen beim Auftragsstart eingefrorenen Stand; Subject und Instanz koennen aus verschiedenen Registry-Staenden stammen; select_in prueft den Snapshot vor SETTINGS_LOCK.
- Beleg: P7-Review Befund 1 (hoch), 2, 8; grep im Planer bestaetigt.
- Konsequenz: P7n — guard liest aktuellen Stand, Freigabe-Aenderung bricht gesperrte Auftraege/Probestimme ab, Subject von der Instanz; Regressionstest.
- Status: erledigt (P7n c21d9fab)

## B8 — P7: Ollama-Probe ohne Cache und ohne check_use; Phonon-Stimme kann still laden (Review 3, 4)
- Beobachtung: probe_speech-Ergebnis wird nach Health-Frist verworfen (nie gecacht), Probe synthetisiert ohne Opt-in/Zustimmung; Phonon --voice mit Katalogname ohne allow_network nicht abgesichert.
- Beleg: P7-Review Befund 3, 4 (mittel).
- Konsequenz: P7n — Probe in eigener Task mit Cache und nur nach check_use; Phonon-Stimmen offline nur default/Pfad/<model>/voices.
- Status: erledigt (P7n c21d9fab)

## B9 — P7: Kleinere Befunde (Review 5, 6, 7, 9 + B6)
- Beobachtung: Cache-Tag ohne base_url; Fish-Bench misst refresh/ensure_server je Synthese mit; Phonon-Stimmenliste innere Frist > aeussere; Benchmark-Ergebnisse-Fallback ueber exe.ancestors liest C:\docs; Lizenznotiz mit leerem Modell (B6).
- Beleg: P7-Review Befund 5, 6, 7, 9.
- Konsequenz: P7n behebt alle; Befund 6 vor dem Messlauf P6.
- Status: erledigt (P7n c21d9fab)

## B10 — Planer: Fish-Lizenz falsch angegeben (Standard-Engine ist nichtkommerziell)
- Beobachtung: Fish-Deskriptor nannte Code Apache-2.0, Gewichte ungeprueft. Lokal belegt: C:\AI\fish-speech\LICENSE und checkpoints\s2-pro\LICENSE.md = "Fish Audio Research License" (Stand 07.03.2026): Forschung/nichtkommerziell frei, kommerzielle Nutzung nur mit separater Lizenz.
- Beleg: Dateikopf beider LICENSE-Dateien, s2-pro README (license_name: fish-audio-research-license).
- Konsequenz: P7n korrigiert Deskriptor (non_commercial, verified 2026-10-26), keine Sperre (Standardverhalten); Owner-Entscheidung O3: Fish fuer kommerzielle Nutzung lizenzieren oder anderen Standard waehlen. Fixture/Doku (licenses.md) nachziehen.
- Status: erledigt (P7n c21d9fab)
