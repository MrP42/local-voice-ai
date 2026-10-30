# Sprachgespräch mit der KI

Echtzeit-Gespräch wie am Telefon: Die KI hört zu, während man spricht, bereitet die Antwort schon in der Sprechpause vor, antwortet mit der eingestellten Stimme (Standard: Anton) und lässt sich unterbrechen. Seite „Sprachgespräch“ in der Seitenleiste (ab Version 0.22.0).

## Ausprobieren

1. Installer starten, App öffnen. Unter **Eingabe** ein Spracherkennungsmodell wählen und die Entladezeit auf „Nie“ oder „2 Minuten“ stellen (sonst lädt das Modell nach jeder Eingabe neu).
2. Unter **Modelle** ein aktives Sprachmodell wählen (Ollama lokal oder ein API-Modell) und unter **Vorlesen** die Stimme prüfen (Anton).
3. **Sprachgespräch → Gespräch beginnen**, Mikrofon erlauben, sprechen.
4. **Headset empfohlen.** Mit Lautsprechern hilft die Echo-Unterdrückung des Browsers, ist aber nicht perfekt; „Unterbrechen erlauben“ bei Echo notfalls abschalten.
5. Oben auf der Seite steht nach jedem Wechsel die **Zeit bis zum ersten Ton** (letzter Laut von Ihnen bis erster Ton der KI). Der Tooltip zeigt die Anteile (Erkennung, erster Satz, Stimme).

## Aufbau

```
Mikrofon (WebView, Echo-/Rauschunterdrückung) → AudioWorklet 16 kHz
   → TurnTaker (Pegel, Rauschboden, Sprechbeginn, Pause, Unterbrechen)
        speech_start ────────────────► Äußerung beginnt (300 ms Vorlauf)
        early_end (≈ 380 ms Stille) ─► Spekulativ: STT → LLM-Strom → TTS je Satz
        end (≈ 700 ms Stille) ───────► Wiedergabe freigegeben
        resume (Nutzer spricht weiter) → Vorarbeit verwerfen
        barge_in (≈ 260 ms Sprache, während die KI spricht) → Wiedergabe stoppen, LLM abbrechen
Kern (Rust, Tauri-Commands):
   voice_call_start        Modelle vorwärmen, Warnungen
   voice_call_transcribe   PCM16 → Text (vorhandene Spracherkennung, CPU/GPU)
   voice_call_chat         aktives Sprachmodell als Strom, Sätze als Ereignis `voice-call`
   voice_call_cancel       Abbruch
   voice_call_synthesize   Satz → WAV (vorhandene Vorlese-Engine, Satz-Cache)
Wiedergabe: gaplose Warteschlange im selben AudioContext wie das Mikrofon
```

Entscheidungen:

- **Gesprächsführung im Frontend**, weil nur dort die Echo-Unterdrückung des Browsers greift (sie braucht die Wiedergabe im selben Audiokontext).
- **Spekulativer Start** bei früher Stille spart die Pausenlänge ab der Wartezeit; Spricht der Nutzer weiter, wird nichts abgespielt.
- **Erstes Häppchen darf am Komma enden** (`sentences.rs`), damit die Stimme früher beginnt.
- **Verlauf enthält nur, was gesprochen wurde**: bei Unterbrechung wird die Antwort dort abgeschnitten.
- Modelle sind austauschbar, weil `voice_call_chat` den aktiven Anbieter aus **Modelle** nutzt (Ollama, OpenAI-kompatibel, Claude-Anbindungen, Abrechnung und Regelwerk wie überall).

## Bekannte Grenzen (MVP)

- Spracherkennung läuft je Äußerung (nicht Wort für Wort). Die Latenz ist dadurch durch Modellgröße und Äußerungslänge bestimmt; kleine Modelle (z. B. Whisper small/Parakeet) sind klar schneller.
- Die Stimme erzeugt je Satz (kein Teil-Audio im Satz).
- Pegelbasierte Sprecherkennung (kein Silero in der Gesprächsschleife). Hintergrundgespräche können auslösen.
- Keine Werkzeuge (Phase 2) und keine Telefonie (Phase 3).

## Phase 2: Werkzeuge

Die KI soll auf Wissensbasis, Aufgaben/To-dos, Poststelle (Mailsuche) und Status zugreifen. Vorschlag:

- Funktionsaufruf im Gespräch über die vorhandene Agentenbrücke/MCP-Schicht (`agent_bridge`, `mcp`), gleiche **lesende Konnektoren** wie die Workflow-App (Loopback-HTTP, sobald entschieden).
- Antworten zu Werkzeugaufrufen werden vorgesprochen („Ich schaue kurz nach …“), damit keine Stille entsteht.
- **Schreibende Aktionen nur mit gesprochener Bestätigung** („Soll ich die Aufgabe ‚…‘ anlegen?“ → ja/nein), im Protokoll sichtbar.
- Mandantentrennung: nur der freigegebene Wissensbereich, keine fremden Kundendaten.

## Phase 3: Telefonie (Skizze)

Ziel: Bot anrufen und vom Bot angerufen werden.

```
Telefon ─ PSTN ─ SIP-Trunk (Anbieter) ─ SBC/PBX (Asterisk oder FreeSWITCH, self-hosted, VPS)
                                              │  RTP/Audio (G.711 8 kHz oder Opus)
                                              ▼
                                   Gesprächsdienst (Brücke)
                         ┌──────── Streaming-STT → TurnTaker → LLM → TTS ────────┐
                         └───────── dieselben Bausteine wie die Gesprächsseite ──┘
```

- **Ankoppeln:** Asterisk `AudioSocket`/`ExternalMedia` bzw. FreeSWITCH `mod_audio_fork`/ESL liefert das Gespräch als PCM-Strom an einen Dienst; derselbe Automat (`turnTaking.ts`) wird dort portiert (Rust oder Node) und nutzt das Sprachmodell/TTS wie hier.
- **Ausgehende Anrufe:** Originate über AMI/ARI bzw. ESL; Kontakt-/Einwilligungsregeln und Rufnummernanzeige beachten.
- **Rechtliches:** Gesprächsaufzeichnung nur mit Einwilligung; Ansage, dass eine KI spricht (EU AI Act Art. 50); keine Inhalte im Log; Anbieter mit EU-Rechenzentrum.
- **Unterschiede zum Desktop:** 8 kHz Telefonband (andere STT-Modelle/Rauschen), Echo kommt vom Netz statt Lautsprecher, Latenz des Trunks (~100–300 ms) kommt dazu, Barge-in per Voice-Activity-Detection statt Browser-Echounterdrückung.
- **Optionen und Kosten sind vor Entscheidung einzuholen:** (a) deutscher SIP-Trunk eines Anbieters mit eigener Rufnummer (monatliche Rufnummerngebühr plus Minutenpreis, typischerweise wenige Cent/Minute — aktuelle Preise beim Anbieter prüfen); (b) Cloud-Telefonie mit Webhook-/Streaming-API (einfach, aber Audio läuft über den Anbieter, teurer pro Minute, Datenschutz prüfen); (c) Fritz!Box/vorhandene Telefonanlage als SIP-Nebenstelle für den Eigenbedarf (kein Trunk nötig, am günstigsten).
- **Empfehlung:** erst (c) bzw. (a) mit Asterisk auf dem vorhandenen VPS als Nebenstelle prototypisieren, dann erst eine eigene Rufnummer.
