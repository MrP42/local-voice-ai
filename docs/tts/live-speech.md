# Live-Sprechwarteschlange (lva#782 A3)

Eigene Warteschlange über der vorhandenen TTS-Pipeline: `ttsSpeakTextFrom`
setzt einen fertigen Text voraus. Eine wachsende Zielfassung müsste dessen
Satzgrenzen, Fortschritt und laufende Sitzung fortwährend umschreiben. Hier
bleibt ein Originalsatz eine Einheit, auch wenn seine Übersetzung mehrere
Sätze oder wegen der Engine-Grenze mehrere Audioteile ergibt.

## Vertrag für A2b

1. Vorhandene Ganzübersetzung hat weiter Vorrang; das entscheidet die UI.
2. Zielstimme über die bestehenden TTS-Einstellungen wählen. Die Queue verwendet
   ausschließlich diese konfigurierte Stimme, keine Sprecherwahl aus Modelltext.
   Bestehende Stimmwechsel gelten am nächsten TTS-Satzende.
3. Auf `tts-speak-progress`, `tts-current-sentence` und `tts-sentence-piece` hören.
4. `tts_live_speak(request)` starten, dessen Promise während der Wiedergabe offen
   bleibt. `text` ist exakt der Originaltext aus A1; `source_offset` ist UTF-16.
   `session_id`, `session_generation`, `page_id` und `target_lang` identifizieren
   die Sitzung. `session_generation` steigt appweit bei einer neuen Reader-Sitzung.
5. Nach dem ersten passenden Fortschrittsereignis (Startbestätigung) mittels
   `tts_live_push(session_id, session_generation, epoch, sentences)` liefern:
   bis zu 40 Objekte `{source_start, source_end, translation}` pro Aufruf.
   Bereits vorhandene A1-Ergebnisse ebenfalls liefern. Die Grenzen müssen exakt
   zu A1 passen; die Queue prüft den gesamten Stapel vor der Übernahme.
6. Die **Sprech-Epoche** steigt bei Sprung, Sprach-/Quellwechsel und Abbruch.
   Sie bleibt bei A1-Vorlauf-Nachbestellungen unverändert. Der Reader prüft zuerst
   die A1-Ergebnisepoche und liefert gültige Ergebnisse dann mit der aktuellen
   Sprech-Epoche. Alte A1-Ergebnisse dürfen nicht auf eine neue Sprech-Epoche
   umetikettiert werden.
7. Ein Positionswechsel startet einen neuen `tts_live_speak`-Aufruf mit höherer
   Epoche. Der alte Lauf wird abgebrochen. Ein geplanter Sprachwechsel am Satzende
   startet dort mit dem bisherigen `source_end`; keine Wortzuordnung zwischen
   verschiedenen Sprachen wird behauptet.
8. `tts_live_cancel(session_id, session_generation, epoch)` verlangt eine höhere
   Epoche und betrifft nur diese Sitzung. Auch `tts_cancel`, Serverstopp oder
   ein neuer gewöhnlicher TTS-Kernauftrag brechen den Lauf ab. Fortsetzen startet
   einen neuen Live-Aufruf mit höherer Epoche; `tts_speak_resume` gilt hier nicht.

Fehlt der nächste Satz, bleibt das Promise offen und die Queue wartet. Sie
überspringt keine Lücke und synthetisiert niemals den Originaltext als Ersatz.
Das Promise endet am Quellende mit Erfolg, bei Abbruch mit `tts_live:cancelled`.
Ungültige Eingaben: `tts_live:invalid_request`; veraltete Lieferung/Start:
`tts_live:stale_epoch`; volle Queue: `tts_live:queue_full`. TTS-Enginefehler werden
wie beim normalen Vorlesen zurückgegeben. Keine neuen Ereignisnamen.

## Ereignisse und Koordinaten

- `tts-current-sentence`: `context: "speak"`, `index` ist der A1-Satzindex im
  **Original**, `text` der Originalsatz, `source_start`/`source_end` UTF-16.
  `duration_ms`, `speech_ms`, `pieces_ms` beschreiben das tatsächlich gesprochene
  Zielaudio. Bei überlangen Übersetzungen beginnt jeder TTS-Teil eine neue Uhr,
  behält aber denselben Originalindex und dieselben Quellgrenzen.
- `tts-sentence-piece`: gleicher Originalindex und gleiche Quellgrenzen,
  `offset_ms` innerhalb des aktuellen TTS-Teils.
- `tts-speak-progress`: `position` zählt vollständig gesprochene Originaleinheiten,
  `total` alle Originaleinheiten, `source_offset` ist beim Start deren Beginn,
  danach das Ende des zuletzt vollständig gesprochenen Originalsatzes.
- Alle drei tragen zusätzlich `session_id`, `page_id`, `epoch`, `target_lang`.
  A1 trennt auch kurze Sätze einzeln; die UI verwendet die Quell-Offsets als
  maßgebliche Zuordnung, nicht die Satzgrenzen einer zusammengesetzten Zielfassung.

Quellstücke und ausstehende Übersetzungstexte leben nur im RAM; höchstens 2000
ausstehende Übersetzungen mit insgesamt 4 MiB, je Satz höchstens 8 KiB. Ende und
Abbruch räumen die Sitzung ab. Synthetisiertes Audio nutzt den bestehenden
TTS-Cache wie gewöhnliches Vorlesen. Keine zusätzlichen Abhängigkeiten oder Logs.

## Tests / Gegenproben für den ausführenden Thread

Nicht ausgeführt (Auftragsvorgabe). Rust-Filter: `managers::tts::live::tests`.
Die Tests verwenden ausschließlich synthetischen Text, keine Engine oder Geräte.

| Test | Gegenprobe: absichtliche Regression, die rot werden muss |
|---|---|
| `source_coordinates_survive_different_translation_lengths_and_splitting` | Zieltextlänge/-index anstelle der Quellkoordinaten im Payload verwenden |
| `missing_sentence_waits_even_when_later_translation_is_ready` | Ersten verfügbaren Satz statt ausschließlich `position` entnehmen |
| `epoch_change_rejects_old_deliveries_and_completion` | Epochenprüfung beim Push bzw. Laufprüfung bei complete entfernen |
| `cancel_wakes_waiter_and_cannot_cancel_a_new_epoch` | Abbruchflag in current ignorieren bzw. alte Cancel-Epochen zulassen |
| `malformed_spans_empty_targets_and_stale_sessions_are_rejected_atomically` | Endgrenzenprüfung entfernen oder den Stapel bereits während der Prüfung einfügen |
| `last_sentence_finishes_only_after_completion_and_duplicates_do_not_repeat` | Position schon beim Entnehmen erhöhen oder laufenden Satz nochmals einreihen |
| `invalid_start_does_not_replace_the_active_session` | Offset-/Sprach-/Generationsprüfung beim Start entfernen |

Build, Testläufe samt roten Gegenproben, Bindings-Export und Audio-/A2b-Integration
sind vom ausführenden Thread noch zu prüfen. Bindings sind hier von Hand ergänzt,
weil kein Cargo-Aufruf erlaubt ist.
