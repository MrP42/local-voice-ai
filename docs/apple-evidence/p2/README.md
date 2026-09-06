# P2 – native Apple-Stabilisierung abgeschlossen

Abnahme am 06.09.2026 im ausdrücklich vereinbarten **Simulatorumfang**. Der native
SwiftUI-Prototyp ist auf dem virtuellen iPhone 15 Pro Max und der Watch Ultra 3
installiert. Getesteter Programmstand: `a9fbf6574c555002272b28ae72e455ff0333f128`,
isolierter Branch `codex/apple-p0-p1`, P2-Ausgangspunkt `cfd180a`.
Desktop-Tauri und bestehende lokale Änderungen bleiben erhalten. Kein Push, keine Veröffentlichung.

## Verhalten und Änderungen

Die Watch speichert Audio dauerhaft, bevor die Aufnahme bestätigt wird. Übergabe,
Antwort und Quittung behalten feste Identitäten. Das iPhone transkribiert und
beantwortet lokal; die Watch spricht die gespeicherte Antwort mit System-TTS.
Whisper und Qwen laufen im iPhone-Prozess des Intel-Simulators, ohne Cloud oder
Host-Inferenzserver. Auf der Watch befindet sich kein Desktop-/ONNX-/Modellstack.

Aufnahme, Transport und Jobs sind getrennt. Persistente Aufträge erlauben maximal
drei automatische Versuche mit verzögertem Retry. Abbruch erhält Original und
Transkript; manuelles Wiederholen setzt das Versuchsbudget zurück. Provider-Timeout
und Benutzerabbruch sind getrennt. Teilweise fehlgeschlagene Initialisierung ist
über die Wiederherstellungsoberfläche erneut möglich.

Beschädigte Einträge bleiben sichtbar und exportierbar. Gesunde Einträge bleiben
verwendbar; vollständige verwaiste Speichertransaktionen werden wiederhergestellt.
Zusätzliche unveränderliche Identitätsdateien ermöglichen sichere Zuordnung.
Nicht zuordenbare alte Originale blockieren neue Annahme vorsorglich, statt eine
Identität zu erfinden. Fehlendes Audio wird konservativ im Speicherbudget gerechnet.
Budgets reservieren Platz vor Aufnahmebeginn; bestätigte Originale werden nie
automatisch zur Platzgewinnung gelöscht. Ein bestätigter Datensatz darf weder
Transkript noch bereits vergebene Antwortidentität verlieren oder ersetzen.

Modelle zeigen Verfügbarkeit und Größe, Base/Small sind ausdrücklich wählbar;
Import prüft exakte Größe und SHA-256 vor atomarem Austausch. Die feste Antwort
bleibt eine auswählbare Diagnoseoption. Digitale Stille und stationäres breitbandiges
Rauschen werden abgewiesen; dies ist kein allgemeiner Sprachaktivitätsdetektor.

## Frische Builds, Tests und Review

- 59 Kerntests bestanden, einschließlich vier zunächst roter Speicherregressionen
  und der Unterscheidung von Timeout und Abbruch.
- 14 tatsächliche Prozessabbrüche an Speicher-/Quittungsgrenzen bestanden.
- Vier native iPhone-UI-Tests und ein Watch-UI-Test bestanden. Die letzte Änderung
  der Komponenten-Erstellungsreihenfolge wurde zusätzlich durch den gezielten
  Wiederherstellungs-UI-Test mit frischem App-Build geprüft.
- 20 Warm-/Kaltturns und 100 weitere lokale Sprachaufträge bestanden. Alle 200
  Originalaudiodateien des 100er-Laufs auf beiden Geräten tatsächlich gelesen und
  mit SHA-256 geprüft; Antwortidentitäten, Transkripte und persistente Jobs auditiert.
- Alle Fälle der Lifecycle-Matrix nach den Review-Korrekturen erneut bestanden.

Nach ausdrücklicher Zustimmung wurden ausschließlich die vier freigegebenen
Quelldateien durch die tatsächliche Claude-CLI ohne Toolzugriff geprüft.
`claude-review.md` enthält den Review des damaligen Quellstands, `claude-triage.md`
die Bewertung aller 17 Punkte. Bestätigte Befunde sind in `4c8ac04`, `71a1d5a` und
`a9fbf65` korrigiert und lokal verifiziert. Dies ist keine Behauptung eines zweiten
Claude-Reviews der korrigierten Version. Es steht keine Freigabe mehr aus.

Ein früherer Duplikat-Harness-Lauf schlug beim Vergleich des Gesamtverlaufs fehl;
welcher Eintrag sich damals änderte, wurde nicht erfasst. Der Harness wartet jetzt
auf abgeschlossene fremde Jobs und protokolliert Differenzen. Der frische Lauf
bestand mit vollständig stabilen Verläufen. Auch der fehlgeschlagene AX-Suchergebnis-
Versuch am App-Umschalter bleibt als Testhistorie erhalten; die richtige Karte
wurde anschließend visuell geprüft und nativ weggewischt.

## Latenzen auf dem Intel-Simulator

Zehn Paare mit derselben deutschen Sprachdatei, vollständiger Watch–iPhone–Watch-Pfad.
Kalt heißt neuer iPhone-Prozess, warm der nächste Turn im selben Prozess. Die Watch
startet jeweils neu; Betriebssystem-Caches bleiben bestehen und Modellkontexte
werden pro Aufruf erstellt. Bei n=10 ist nearest-rank-p95 zugleich das Maximum.

| Messung | Kalt Median / p95 | Warm Median / p95 |
|---|---:|---:|
| STT | 8,109 / 8,537 s | 7,320 / 7,859 s |
| Antworterzeugung | 8,803 / 9,182 s | 6,343 / 6,574 s |
| Übertragungsquittung | 4,959 / 5,948 s | 4,913 / 5,213 s |
| E2E bis gespeicherte Watch-Antwort | 24,022 / 24,841 s | 20,521 / 21,767 s |
| E2E bis tatsächlicher TTS-Start | 24,216 / 25,047 s | 20,718 / 21,955 s |

E2E beginnt am Aufnahmeende beziehungsweise an der gesicherten Testfixture.
TTS-Endpunkt ist der erste AVSpeechSynthesizer-Startcallback, nicht vollständiges
Abspielen. Quantile stammen aus Einzelzeiten, nicht addierten Stufenquantilen.
Fixture-Läufe umgehen das Mikrofon. Die frische echte Watch-Recorderprobe bei
Wrist Down maß **1.384 ms Aufnahmefeedback und 274 ms Persistenz** (n=1).
Frühere P2-Recorderproben (verschiedene Zwischenstände, je n=3) ergaben median
818/769 ms Feedback und 554/537 ms Persistenz auf iPhone/Watch; sie sind ausdrücklich
historische Zusatzdaten, keine neue Stichprobe des finalen Programms.

Der 100er-Stapel dauerte 953,768 s, einschließlich Hintergrund, Watch-Neustart,
iPhone-Neustart und Simulator-Shutdown/Wiederverbindung. Median/p95: STT
5,101/5,303 s, Generierung 2,824/2,997 s, Übertragungsquittung 1,992/4,005 s.
Der größte Übertragungswert von 56,003 s bleibt enthalten. Die kürzere Sprachdatei
ist nicht direkt mit den Warm-/Kaltturns vergleichbar. E2E einschließlich Warteschlange:
546,998/888,551 s bis gespeicherter Antwort (n=100), 562,538/897,418 s bis TTS-Start
(n=96). **100 Antworten gespeichert, 96 TTS-Starts erfasst**; die anderen vier
werden nicht als abgespielt gezählt. Gespeicherte Antworten sind erneut abspielbar.
Diese Warteschlangenwerte sind keine normale Einzelturn-Latenz.

## Wann Verarbeitung funktioniert oder wartet

Bei aktiven Apps und erreichbarem Gegenüber läuft die gesamte Verarbeitung durch.
Die ursprünglichen Echtzeitziele werden auf diesem Intel-Simulator trotzdem verfehlt;
auch im warmen Einzelturn sind etwa 20,7 s bis Sprachbeginn gemessen.

Bei Hintergrund, Sperrbildschirm oder benutzerseitigem Beenden bleibt die Aufnahme
erhalten und die Oberfläche zeigt „gespeichert – Verarbeitung folgt“. In den
zehnsekündigen Beobachtungsfenstern entstand keine Antwort; nach Aktivierung wurde
dieselbe Aufnahme verarbeitet. Beim Wegwischfall war sie zunächst nur auf der Watch.
WatchConnectivity kann später zustellen; dauerhafte Hintergrund-Inferenz wird nicht
versprochen. Wiederverbindung und Neustart erhalten Identität und Quittungen.

Native CPU-Abbrüche wurden nach 4,285 s STT beziehungsweise 0,105 s Generierung
beobachtet; derselbe Auftrag wurde ohne erneute Transkription fortgesetzt.
Abbruch ist kooperativ, Modellladen nicht hart unterbrechbar.

## Qualitäts- und Hardwaregrenzen

Die zwölf synthetischen deutschen Qualitätsfälle des vorangehenden P2-Quellstands
prüften Umlaute, Zahlen, Absatzwörter, Fakten/Rechnen, Aktionswünsche, Negation,
Stille und Rauschen. Die betreffenden Qualitätsfilter und CPU-Provider wurden durch
den Review nicht geändert. Qwen antwortete auf korrekt transkribiertes „7 + 5“ mit
„8“: ein bekannter, nicht behobener Qualitätsmangel. Der begrenzte deutsche
Aktionsfilter garantiert keine allgemeine semantische Sicherheit.

Der historische Vergleich derselben fünf P1-Audiodateien ergab Base 14/21 wörtliche
Schlüsselwörter bei 7,659 s median STT, Small 16/21 bei 27,164 s. Das ist keine WER,
berücksichtigt gleichbedeutende Zahlenschreibweisen nicht und kein neuer Small-Lauf.

**Akkuauswirkung unbekannt.** Der Simulator erlaubt keine belastbare Akku-Prozentmessung
auf Watch/iPhone. Ebenso fehlen physischer Funk, verschlüsselte Dateien im echten
Sperrzustand und Anruf-/Bluetooth-Routen. Die Audiounterbrechung wurde nach echtem
TTS-Start als Notification injiziert. Apple Speech/Foundation Models sind in dieser
Intel-Simulatorumgebung nicht verfügbar. Diese Grenzen bleiben offen dokumentiert
und sind kein noch ausstehender Teil des ausdrücklich virtuellen Auftrags.

Folgeprioritäten: Antwortqualität, Zielgerät-Latenz, gesonderte Hardwareabnahme und
drei vergleichbare 8-h/30-Turn-Energiemessungen. Details in `next-priorities.md`.
Rohdaten und Protokolle unter `results/`, `logs/`, `screenshots/`; aktuelle Laufzeit-
Nachweise tragen `after-review`, ältere Dateien bleiben als Historie erhalten.
