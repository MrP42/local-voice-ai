# Tatsächlicher Claude-Review und lokale Prüfung

Am 06.09.2026 nach ausdrücklicher Benutzerfreigabe mit Claude Code durchgeführt.
Übermittelt wurden ausschließlich Store.swift, Envelope.swift, VoiceModel.swift und
LocalProviders.swift, ohne Tools und im Safe Mode. Rohantwort: `claude-review.md`.
Keine behauptete Prüfung der nicht übermittelten Dateien durch Claude.

| Nr. | Bewertung im vollständigen lokalen Kontext |
|---|---|
| 1 | Bewusste Sperre nur bei **unbekannter alter Identität**: Ein solcher Eintrag darf nicht durch eine womöglich doppelte Nachricht übergangen werden. P2-Einträge haben identity.json; der Korruptionsprobe fehlt deshalb die behauptete globale Wirkung. Aufnahmeentwürfe haben keine sessionId. Gesunder vorhandener Verlauf bleibt verarbeitbar. |
| 2 | Bestätigt, defensiv korrigiert: eine Antwort darf ein vorhandenes Transkript weder löschen noch ändern. Regression war rot und ist danach grün. |
| 3 | Bestätigt, defensiv korrigiert: eine bereits vergebene Antwort-ID bleibt unverändert; eine abweichende eingehende ID wird abgewiesen. Regression rot → grün. |
| 4 | Bestätigt und korrigiert: Komponenten werden idempotent eingerichtet; der sichtbare Retry kann eine unvollständige Einrichtung reparieren. Recovery läuft nach Verdrahtung der Callbacks. Eigener Simulator-UI-Fehlerprobe hinzugefügt. |
| 5 | Korrigiert: besessener Beobachter mit Abmeldung beim Freigeben. |
| 6 | Nicht übernommen: answered bedeutet **dauerhaft gespeicherte Antwort**, nicht vollständig abgespieltes Audio. Die Antwort ist schon vor TTS gespeichert/quittiert. Wiederanhören bleibt in der UI möglich; didFinish ist keine Voraussetzung für sichere Zustellung. |
| 7 | Kein erreichbarer Fehler über die vorhandenen Job-APIs: pause/fail/retry bearbeiten keine bereits beantworteten Einträge. completed ist für eine gespeicherte Antwort korrekt. |
| 8 | Bestätigt: Provider-Timeout wird jetzt ausdrücklich von Benutzer-/Scene-Cancellation getrennt. Kerntest prüft Wartezustand und begrenztes Retry statt Pause. Die Apple-APIs selbst sind auf dem Intel-Simulator weiterhin nicht verfügbar. |
| 9 | Durch ausgelassenen Kontext widerlegt: beide nativen Bridges besitzen 90-s-Deadline und Abbruchcallbacks. Modellladen bleibt kooperativ begrenzt; kein harter Thread-Kill behauptet. |
| 10 | Bestätigt: Aufnahmebeginn reserviert zusätzlich Platz im bestätigten Audiobudget. Regression rot → grün. |
| 11 | Retention ist Benutzeranforderung, keine automatische Löschung unbestätigter Originale. Bestätigt war die Cache-Abweichung bei fehlgeschlagener Löschung einer redundanten Datei: Cache-Issue verschwindet jetzt nur, wenn die Datei tatsächlich weg ist. |
| 12 | Kein gezeigter Race: VoiceModel, CaptureController, VoiceTransport und JobProcessor verwenden denselben MainActor. Nur die Inferenz läuft außerhalb; sie erhält keine Store-Instanz. |
| 13 | Defensiv korrigiert: ein laufender Sprachversuch hält jetzt auch die Utterance selbst, bis Finish/Cancel aufräumt. |
| 14 | Defensiv verbessert: erste E2E-Zeit wird anhand des aktuellen Store-Eintrags geprüft, nicht nur des UI-Snapshots. |
| 15 | Kein nachgewiesener Wiederholungsweg: der Worker überspringt vorhandene Antworten; Abschlussänderung trägt die ID einmal, das Drain-Ende nil. Transport-Duplikate sprechen nicht erneut. Manuelles Wiederanhören bleibt beabsichtigt. |
| 16 | Version ist absichtlich verpflichtend. timings existiert seit dem ersten P1-Schema. Additive spätere Felder sind optional. Kein vorhandener Altbestand ohne timings nachgewiesen. |
| 17 | Keine fehlerhafte Übertragung: Fabrik überschreibt die Felder vollständig, Roundtrip-Tests bestehen; unvollständige Reply-Envelopes werden abgewiesen. |

Zusätzlicher bestätigter Speicherbefund aus der Review-Nachprüfung: Eine fehlende
Audiodatei ließ die Quotenberechnung für alle neuen Aufnahmen fehlschlagen. Jetzt
bleibt der beschädigte Eintrag sichtbar und reserviert konservativ 1 MiB; andere
Aufnahmen können bei freiem Budget angenommen werden. Regression rot → grün.

Der externe Review ist abgeschlossen. Bestätigte Änderungen werden durch frische
Kern-, Build-, UI- und Transportprüfungen abgesichert; siehe Ergebnisbericht.

Der erste post-Review-Duplikat-Harness meldete eine Änderung am gesamten Verlauf,
ohne die betroffene ID zu protokollieren. Seine Ausgangslage enthielt möglicherweise
noch nicht abgearbeitete Aufnahmen aus den UI-Tests; die genaue erste Abweichung
lässt sich daraus nicht rekonstruieren. Der Harness wartet jetzt auf abgearbeitete
Aufträge und speichert bei einer erneuten Abweichung die IDs. Der Wiederholungslauf
ist mit unveränderten vollständigen Verlaufsidentitäten bestanden.
