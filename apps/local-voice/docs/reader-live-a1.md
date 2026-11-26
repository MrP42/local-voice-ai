# Reader-Live-Übersetzung: A1-Vertrag (#782)

Implementierung ohne UI und ohne neue Abhängigkeiten. `bindings.ts` wird erst im
Tor erzeugt. Keine neuen Prozesse, Hooks, Downloads oder Installationen.

## Tauri-Befehle

`reader_live_translate(request: LiveRequest) -> Result<LiveResult, String>`

`LiveRequest` enthält:

| Feld | Typ | Bedeutung |
| --- | --- | --- |
| `session_id` | String | Vom Reader erzeugte Sitzungskennung, maximal 200 Bytes |
| `page_id` | String | Quellseite, maximal 200 Bytes |
| `text` | String | Exakt angezeigtes Original, maximal 16 MiB |
| `target_lang` | String | Sprachkennung aus Buchstaben, `-`, `_`, maximal 35 Bytes |
| `source_offset` | u32 | UTF-16-Offset im Original |
| `epoch` | u32 | Streng steigend pro Sitzung, auch für jede Nachbestellung |
| `rate` | f64 | Lesetempo, endlich und zwischen 0.1 und 4.0 |

Rückgabe als mit `kind` markierte Union:

- `{ kind: "existing", translation: string }`: vorhandene Ganzübersetzung aus
  dem TTS-Cache, vor Modellwahl und Satzereignissen. Auch Nutzerkorrekturen haben
  dadurch Vorrang. A2 wählt vorhandene andere Buchfassungen weiterhin über den
  bestehenden Fassungswechsel; A1 ordnet deren Text nicht künstlich Sätzen zu.
- `{ kind: "window", sentences: number }`: Vorlauffenster fertig, einschließlich
  der aus dem Arbeitsspeicher gelieferten Sätze. Am Textende ist die Zahl null.

Der Invoke bleibt bis zum Ende des Fensters offen. Vorher den Ereignislistener
registrieren; jeder Satz wird sofort nach seiner vollständigen Antwort geliefert.
Kein Token-Streaming und keine persistierte Live-Fassung.

`reader_live_cancel(session_id: String, epoch: u32) -> bool`

Bricht nur die passende Sitzung mit einer **neueren** Epoche ab, verwirft deren
Arbeitsspeicher und lässt ein laufendes HTTP-Future fallen. `false` bei fremder
Sitzung oder veralteter Epoche. Ein verbleibender Sitzungseintrag ohne Buchtext
hält die letzte Epoche fest. Er verhindert verspätete Aufrufe derselben Sitzung.

## Ereignis

`reader-live-sentence` (`ReaderLiveSentence`, in `collect_events!` registriert):

```ts
{
  session_id: string;
  page_id: string;
  epoch: number;
  target_lang: string;
  source_start: number;
  source_end: number;
  translation: string;
}
```

Offsets sind UTF-16 im unveränderten **Original**, Ende ausschließlich. Auch bei
Emoji oder kombinierenden Zeichen werden keine Übersetzungskoordinaten verwendet.
Die Einheiten entsprechen einzelnen Sätzen; kurze Sätze werden nicht verbunden,
lange nicht am Zeichenlimit geteilt. Absatzgrenzen begrenzen ebenfalls Einheiten.

Ein Offset innerhalb eines Satzes bestellt diesen Satz mit. Für einen Sprachwechsel
am Satzende übergibt A2 das Ende des noch laufenden Originalsatzes und erhöht die
Epoche. A2 verwirft Ereignisse anderer Sitzungen/Epochen auch auf Empfängerseite:
bereits versandte Ereignisse können noch in der Webview-Warteschlange liegen.

## Ablauf und Grenzen

- Ein aktiver Reader je App. Vorlauf: mindestens sechs Sätze und geschätzte
  30 Sekunden (15 UTF-16-Einheiten/s bei Tempo 1), maximal 40 Sätze. Am Buchende
  entsprechend weniger. A2 bestellt bei fortschreitender Quellposition nach.
- Ein Satz pro Modellaufruf, in Lesereihenfolge; der erste Satz wartet auf keinen
  Stapel. Höchstens ein Fenster arbeitet gleichzeitig. Neue Epochen unterbrechen
  laufende und wartende Fenster; alte Texte und Fehler werden nicht ausgegeben.
- Cache nur im RAM, maximal 2000 Sätze. Gebunden an Sitzung, Seite, exakten
  Quelltext, Zielsprache und Modellroute. Verdrängung nach Quelloffset.
- Das aktive lokale Modell hat Vorrang, sonst das erste aktivierte lokale Modell
  einer aktivierten Verbindung. CLI und entfernte HTTP-Adressen sind ausgeschlossen,
  unabhängig von Anbietername oder aktivem Cloud-Standard. Das eingebaute Modell
  muss bereits mit passender Modellkennung laufen. Kein automatischer Modellstart.
- Versand direkt über `llm_versand::versenden` mit frischer Freigabeprüfung,
  Compliance/Budget und Zweck `Translation`. Keine Cloud-Freigabe in A1;
  ohne lokales Modell `reader_live:no_local_model`, ohne Cloud-Rückfall.
- Leere und abgeschnittene Antworten werden nicht als fertiger Satz ausgegeben.
  Ein Fehler stoppt das Fenster; fertige Sätze bleiben im RAM. Erneutes Bestellen
  mit neuer Epoche verwendet sie wieder. Keine Inhaltstexte in Logs/Fehlern.
- Fehlercodes: `invalid_request`, `invalid_rate`, `source_invalid`, `stale_epoch`,
  `cancelled`, `no_local_model`, `permission_unavailable`, `translation_failed`,
  `unusable_reply`, `event_failed`, jeweils mit Präfix `reader_live:`.
- Die bestehende TTS-Satztrennung behält ihre Mindestlänge 15. Nur der neue
  Live-Einstieg verwendet einzelne kurze Sätze sowie Schlusszitate/CJK-Satzzeichen.

## Prüfung durch Thread-Claude / Tor

Hier wurden keine Rust-Bauten oder Rust-Tests gestartet. Die folgenden Läufe
und insbesondere die roten Gegenproben sind noch offen:

```text
cargo test --lib reader_live::tests -- --test-threads=1
cargo test --lib managers::tts::protocol::tests -- --test-threads=1
```

Synthetische Tests ohne Dienste, Netz, Prozesse oder Hooks. Gegenproben einzeln
in einer temporären Kopie durchführen und danach denselben Test erneut mit dem
unveränderten Stand grün prüfen. Keine Mutation im gemeinsam genutzten Baum.

| Gegenprobe (nur temporäre Kopie) | Erwartet roter Rust-Test |
| --- | --- |
| `split_live_sentences` ruft wieder Mindestlänge 15 auf | `kurze_saetze_bleiben_einzeln_und_offsets_sind_utf16` (zusätzlich direkte Gegenprobe mit bisherigem Schnitt im Test) |
| In `units` `.encode_utf16().count()` durch `.chars().count()` ersetzen | `kurze_saetze_bleiben_einzeln_und_offsets_sind_utf16` |
| Vorlauf auf sechs Sätze begrenzen oder Obergrenze 40 entfernen | `vorlauf_hat_mindestzahl_zeit_und_harte_obergrenze` |
| `Slot::current` nur auf vorhandene Sitzung prüfen | `epoche_verwirft_spaete_ergebnisse_und_abbruch_auch_spaete_fehler`, `wechsel_waehrend_antwort_verwirft_text_und_fehler` |
| Abbruchbenachrichtigung in `LiveState::cancel` entfernen | `abbruch_laesst_laufendes_future_fallen_und_gibt_arbeiter_frei` (eingebautes Zwei-Sekunden-Zeitlimit) |
| Cache-Begrenzung oder Quell-/Sprachvergleich in `Slot::enter` entfernen | `cache_ist_begrenzt_und_an_quelle_und_sprache_gebunden` |
| Lokalprüfung in `local_target` entfernen | `nur_http_loopback_und_passendes_laufendes_modell` |
| Lokalen Kandidatenfilter in `route` entfernen **und** lokale Zielprüfung umgehen | `cloud_standard_faellt_nur_auf_lokal_zurueck_und_nie_umgekehrt` |
| `prefer_existing` immer den Live-Zweig ausführen lassen | `ganzuebersetzung_hat_vorrang_vor_modellwahl_und_ereignissen` |
| `usable_reply` leere/abgeschnittene Antworten akzeptieren lassen | `leere_und_abgeschnittene_antworten_werden_nicht_ausgegeben` |
| Cache-Lesezugriff in `run_window` durch `None` ersetzen | `ein_ereignis_je_satz_cache_und_abbruch_ohne_transport` |

Zusätzlich offen: Rust-Check, gezielte Rust-Formatprüfung, Bindings-Export im Tor,
Integration mit einem laufenden lokalen Modell und A2-Reader inklusive Vorlesen.
