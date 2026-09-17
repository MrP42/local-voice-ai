Kontext ist klar: Sprechstift ist ein Handy-Fork (Tauri 2: Rust-Backend mit lokalem Whisper/Parakeet-STT über Vulkan, React-Frontend, Hotkey→Aufnahme→VAD→Transkript→Paste), Branch M3 hat gerade den Paste-Pfad mit Testharness stabilisiert. Deine how-to-fish.md im Repo-Root zeigt, dass die Fish-Integration hier landen soll. Wichtig fürs Design: llm_client.rs (LLM-Postprocessing) und rodio (Audio-Playback) existieren bereits — zwei Bausteine, die wir wiederverwenden können.

Scope-Einschätzung nach Brainstorming-Skill: Das ist ein Fünf-Subsystem-Vorhaben und muss zuerst dekomponiert werden. Mein Vorschlag (jedes Teilprojekt = eigener Spec → Plan → Umsetzung → Test, mit anfassbarem Artefakt am Ende):

#	Teilprojekt	Kern	Neu/vorhanden
TP1	TTS-Fundament „Vorlesen“	Rust-Client für den Fish-Server (127.0.0.1:8080), Settings, Health-Status, Text/Clipboard vorlesen per Hotkey, Playback via rodio	komplett neu — Fundament für alles Weitere
TP2	Stimmen klonen	Referenz-Aufnahme in der App (Mikro + STT liefert das Transkript automatisch!), Verwaltung, aktive Stimme, reference_id	neu, nutzt TP1
TP3	Audio-Übersetzung	Sprache → Text (vorhanden) → Übersetzung via llm_client → Ausgabe in geklonter Stimme	Pipeline aus Bausteinen
TP4	Stimmwechsler	Aufnahme/Datei → STT → TTS mit Zielstimme (Kaskade, offline)	neu, nutzt TP1–TP2
TP5	Performance/Streaming	triton-windows + --compile am Fish-Server, echtes Streaming-Playback, TTFA senken	Infra, macht TP3/TP4 erst flüssig
Zwei ehrliche Rahmensetzungen dazu: STT bleibt Whisper/Parakeet — fishaudio bietet kein lokales Open-Source-STT-Modell an (die fish.audio-STT ist Cloud-only), und deine lokale Engine ist bereits sehr gut. Und ein Echtzeit-Stimmwechsler ist bei RTF ~6 physikalisch nicht drin — TP4 ist „Datei/Aufnahme umsprechen“, Echtzeit würde erst nach TP5 neu bewertet.

Aufwandsschätzung (CLAUDE.md-Pflicht): Gesamtvorhaben grob 800–1500 kTok über mehrere Etappen; TP1 allein ~150–250 kTok. Zwischenstand melde ich je Etappe bei 50/80 %.