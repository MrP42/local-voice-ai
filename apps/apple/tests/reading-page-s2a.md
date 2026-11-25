# S2a: Übergabe an S2b und Apple-Prüfung

## Entscheidungen für den PR

- `ReadingPageEditor` in VoiceCore bearbeitet Aufnahmen und prüft die gespeicherte
  Zusammensetzungsbasis. Text und Aufnahmefolge werden gemeinsam atomar gesichert.
  Abweichender Seitentext öffnet immer `CompositionChoice`.
- Aufnahmefolge und Löschungen gelten sofort. Bei einer Textkollision ersetzt
  `.replace` den Seitentext durch die gesamte neue Zusammensetzung. `.append`
  hängt bei neuen Aufnahmen nur deren Text, bei Neu-Erkennung nur deren Ergebnis,
  bei Umsortieren/Löschen die verbleibende Zusammensetzung an. `.cancel` lässt den
  Seitentext stehen und verwirft vorgemerkte Neu-Erkennung. Originalbilder bleiben
  wie in S1 erhalten. Veraltete Auswahlen werden abgelehnt.
- Der Player erhält ein flüchtiges `LibraryBook` mit `page:<id>`, direkt aus der
  Seite und ohne Import/Buchdatei. So bleiben Stimme, Mitlesen und Positionsmigration
  im bestehenden Player. Seiten haben keine Hörbuchquelle. `PhoneLibrary` leitet
  ihren Lesestand vor dem normalen Fortschrittspfad in eigene lokale UserDefaults
  um; diese Werte gelangen nicht in `fortschritt.json` oder dessen Sync-Schlüssel.
- Änderungszeiten liegen nur im lokalen Store-Datensatz. Der Seitentext und das
  Desktop-/Sync-Schema bleiben unverändert; Altbestand ohne Zeit wird stabil nach
  ID hinter neueren Änderungen eingeordnet.
- `TextIntakeModel.routesToReadingPages` bleibt **false**, bis S2b die Seiten
  erreichbar macht. Danach auf **true** setzen. Der neue Zweig legt eine Seite an,
  schließt das Prüfblatt und veröffentlicht erst in `sheetDismissed()` dessen
  `openPageID`. S2b navigiert dorthin und setzt die ID nach Verbrauch zurück.
- `after: nil` hängt ans Ende an. Eine inzwischen entfernte markierte Aufnahme
  wird abgelehnt; es wird nicht an einer geratenen Stelle eingefügt.

## Ausgeführt auf Windows

`python -B apps/apple/scripts/test_reading_page_project.py` und
`python -B apps/apple/scripts/test_reading_page_logic_contracts.py`: sechs Tests
einschließlich Gegenproben durch Entfernen/Ändern der Anschlüsse im Speicher.
Diese Prüfungen testen Projektzuordnung und statische Anschlüsse, kein Swift-Verhalten.

## Noch auf Apple auszuführen

`swift test --package-path apps/apple/VoiceCore` sowie iPhone-Build und Laufzeitprüfung
von OCR, Prüfblattwechsel und Player. Keine Swift-Kompilation auf Windows erfolgt.
`ReadingPageEditingTests` enthält synthetische Verhaltens-/Persistenztests.
Für die roten Gegenproben jeweils genau die genannte Sicherung in einer isolierten
Testkopie entfernen, dann den zugeordneten Test ausführen und die Sicherung wiederherstellen:

| Gegenprobe | Erwartet roter Test (Suffix nach `test`) |
|---|---|
| Kollisionszweig in `change` entfernen | `MoveAndDeleteComposeOnlyWithoutEdits`, `RerecognitionRequiresChoiceAndCancelKeepsEditedText` |
| Einfügeindex immer ans Ende legen | `AppendAfterMarkedCaptureAndComposeWithoutEdits` |
| Gültigkeitsvergleich für Text bzw. Aufnahmen in `resolve` entfernen | `StaleChoiceCannotOverwriteNewerText`, `StaleChoiceCannotUndoAnotherCaptureChange` |
| Grabsteinprüfung in `page` entfernen | `LateRecognitionAfterDeletionDoesNotResurrectPage` |
| Lokalen Fortschritt nicht schreiben | `PageProgressSurvivesReloadAndStaysOutsideBookProgress` |
| Sortierung nach `modifiedMs` entfernen | `ModificationOrderAndCompositionBaselineSurviveReload` |
| Zeichen-/Aufnahmelimit entfernen | `LimitsRejectInsteadOfTruncatingAndKeepOriginalText` |

Swift-Gegenproben sind vorbereitet, aber hier nicht ausgeführt. Der tatsächliche
Player-Aufruf ist nur statisch abgesichert; ein iPhone-Lauf muss zusätzlich zeigen,
dass die Zahl der Bibliotheksbücher beim Öffnen einer Seite gleich bleibt.
Projektdatei auf Apple mit `generate_project.py` aktualisieren; der Generator wurde
hier wegen der Vorgabe „Schreiben nur mit apply_patch“ nicht ausgeführt.
PR, neuer Commit-SHA und Meldung im Thread bleiben beim aufrufenden Claude-Thread.
