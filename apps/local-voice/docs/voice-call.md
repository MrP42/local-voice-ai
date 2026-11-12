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
- **Ergänzen und Korrigieren (lva#485)**: Spricht der Nutzer über die laufende Antwort hinweg, hält der Pegel-Barge-in die Wiedergabe nur an (`SpeechPlayer.pause()`, hörbar sofort still); die Erzeugung läuft weiter. Erst der erkannte Text entscheidet (`ergaenzen.ts`, Deutsch und Englisch):
  - Rückmeldelaut („mhm“, „ja“, „okay“, „yeah“, „got it“ …) setzt die Wiedergabe fort. Das gilt nicht, wenn der gerade oder zuletzt gesprochene Satz eine Frage war: dann ist „ja“ die Antwort und ein Beitrag.
  - Ein Stoppwort („stopp“, „halt“, „warte“, „moment“, „stop“, „wait“, „hold on“, auch mit „bitte“, „mal“, „okay“ …) beendet die Antwort ohne neuen Beitrag.
  - Der erste Inhaltssatz endet mit dem Stück, das ihn schließt (Satzzeichen `.` `!` `?` `…` am Stückende, `schliesstSatz`), nicht mit dem ersten Stück: „Für morgen,“ ist kein Satzende. Ohne Satzzeichen schließt das letzte Stück der fertig erzeugten Antwort den Satz (`pruefeSatzende`).
  - Rede vor dem Ende des ersten Inhaltssatzes wird mit der vorherigen Äußerung zu einer („<alt> <neu>“, auch als Kette). Spricht der Nutzer danach weiter, bevor der neue Schritt hörbar wird, bleibt der Vortext erhalten (`Turn.prefix`/`carry`).
  - Spätere Rede bleibt ein neuer Beitrag, und das Modell bekommt im nächsten Aufruf einen kurzen Systemhinweis, dass es unterbrochen wurde (nur auf dem direkten Weg und über den Orchestrator-Verlauf; ein Denker bekommt ihn nicht).
  - Im Verlauf, in der Anzeige und beim Modell steht nach einer Unterbrechung nur, was zu Ende gespielt wurde, plus der angespielte Satz (`gehoert()`), dann „ …“. „Gehört“ heißt tatsächlich begonnen, nicht eingeplant: der Player meldet je Häppchen `onStart`, sobald die Startzeit der Quelle die Uhr des Audio-Kontexts erreicht hat (`SpeechPlayer.syncStarts()`, auch beim Anhalten); ein eingeplantes, noch stummes Häppchen zählt nicht, bei „Stopp“ vor dem ersten Ton bleibt der Verlauf leer. Was während der Pause weiter erzeugt wurde, fehlt.
  - Damit kein halber Beitrag im gespeicherten Gespräch bleibt, meldet die Engine die fertige Antwortzeile (und damit das Speichern von Äußerung und Antwort) erst, wenn der erste Satz zu Ende gesprochen ist; bis dahin ersetzt eine Ergänzung beide still, nimmt die Messung des ersetzten Schritts zurück und lässt „wiederholen“ dessen Antwort vergessen. Vorlagen, S0-Antworten und Bedienung („lauter“) werden nie zusammengeführt.
  - Die Haltezeit zählt weder zu `firstSoundMs` noch zu `fertig_ms`.
  - Fehler der Erkennung beim Anhalten: Wiedergabe läuft weiter, Meldung „Ich habe dich nicht verstanden.“. Dauert sie länger als `TUNING.holdSttMaxMs` (8 s), läuft die Wiedergabe ebenfalls weiter; kommt der Text danach, wird er wie ein spätes Unterbrechen behandelt (Rückmeldelaut: nichts, Stoppwort: anhalten, sonst neuer Beitrag), nach dem Ende des Schritts als gewöhnlicher Beitrag. Das gilt nur, solange die Lage dieselbe ist (`Engine.lage`: neue Äußerung, neuer Schritt oder neue Erkennung zählen hoch); sonst wird das späte Ergebnis verworfen und bricht nichts ab.
- **Inhaltszugriff atomar (lva#509)**: „Bücher, Wissensbereiche und Transkripte freigeben“ ruft `voice_call_inhaltszugriff_lesen` (Stand samt `version`) und genau einmal `voice_call_inhaltszugriff_setzen(erwartete_version, enabled, areas, read_transcripts)`. Der Kern (`app_context/berechtigung.rs`) vergleicht die Version unter einer Sperre, schreibt Hauptschalter, Bereiche und Transkripte in einem Zug und erhöht die Version; stimmt sie nicht, schreibt er nichts (`konflikt`), die Oberfläche liest neu und meldet „Die Einstellung wurde inzwischen geändert.“. Ein Zurückrollen gibt es nicht, ein Widerruf gewinnt immer. Jede Änderung von Hauptschalter, Bereichen, Transkripten oder Notizbuch-Abschnitten erhöht die Version, gleich über welchen Updater (`settings::write_settings`). Ein Abruf merkt sich die Version beim Start und liefert bei Änderung bis zur Ausgabe keine Auszüge (`ContextHits::an_version_gebunden`, Marker `berechtigung_geaendert`); das Tor (`admissible`), `app_context_message` und `app_context_verify` verlangen die unveränderte Version. Der generische Schreibpfad (`write_settings`/`try_write_settings`) darf diese vier Werte nur einschränken (Schalter UND, Listen Schnittmenge, `nur_einschraenken`), nie erweitern; Erweitern geht nur über die gesperrten Updater und `voice_call_inhaltszugriff_setzen`. Ein veralteter Schreibzyklus bringt so keine widerrufene Freigabe zurück, nimmt aber eine frische Freigabe wieder weg, wenn sein alter Stand sie nicht kannte (die sichere Richtung).
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
