# Mobile Gespräche, lokale Auswertung und gekoppelte Geräte

Explizite Erweiterung des Benutzerauftrags vom 06.09.2026. Der Apple-Worktree bleibt
isoliert; erforderliche Desktop-Integrationen werden hier entwickelt, nicht im
ursprünglichen Checkout mit dessen lokalen Änderungen. Kein Push/Release.

## Beauftragter Umfang

- [x] Audio-/Videoimport auf iPhone, dauerhaftes Original und fortsetzbares lokales STT.
- [x] Strukturierte lokale Protokolle gemäß Desktop: Zusammenfassung, Kontext,
  Entscheidungen, Aufgaben/Zuständige/Termine, nächste Schritte, Empfehlungen,
  offene Fragen. Originaltranskript bleibt daneben zugänglich.
- [ ] Redeanteile aus belegten Segment-/Kanalzuordnungen; keine vom LLM erfundenen
  Prozentwerte. Gemischter Ton ist nicht automatisch ein einzelner Sprecher.
- [x] Kopieren, Teilen, Export von Transkript und Auswertung.
- [ ] Verschlüsselte Synchronisierung ausdrücklich gekoppelter Desktop-/Mobilgeräte
  im lokalen Netz, mit Offline-Erhalt, Idempotenz und Konflikterhalt.
- [ ] Watch-/iPhone-Mikrofone als Desktop-Eingabe; vor Aufnahmebeginn gewähltes Ziel
  bleibt bis zum sicheren Abschluss fixiert.
- [ ] Automatische Zielwahl aus aktueller nutzbarer Aktivitätsinformation sowie
  manueller Override. Keine Behauptung globaler iOS-Aktivitätserkennung.
- [ ] Frische Tests, Simulator-/Desktop-Abgleich, Grenzen dokumentieren, lokale Commits.

## Plattformgrenzen und Ausgangspunkt

Apple stellt ReplayKit für die Aufnahme der eigenen App bereit. Ein allgemeiner
Live-Mitschnitt fremder Telefon-/Teams-/WhatsApp-Anrufe ist damit nicht belegt.
Verfügbare Anrufaufzeichnungen können aus Apples Notizen/Dateien importiert werden;
Verfügbarkeit der Apple-Aufnahmefunktion hängt von Region/Sprache ab. Es wird keine
nicht vorhandene Anrufzugriffsfunktion als erfolgreich implementiert ausgegeben.

Desktop-Protokollschema: `managers/meetings/minutes.rs`; Redeanteile:
`managers/meetings/stats.rs` (Kanalzeiten, keine Sprecherdiarisierung).
Apple-Kurzantwort bleibt unverändert nutzbar. Lange Medien benötigen einen separaten
Speicher-/Jobpfad: vorhandene Watch-Pakete sind bewusst auf 1 MiB / kurze Turns begrenzt.

Aktivität ist ein Routing-Signal, keine automatische Mikrofonfreigabe. Aufnahme-
Buttons starten weiterhin nur nach ausdrücklicher Bedienung. Kopplung authentifiziert
Geräte; Netzwerkadressen allein berechtigen keinen Datenzugriff. Ein Verbindungsabbruch
verwirft keine bestätigte Aufnahme. Neue Modelle werden nicht stillschweigend geladen.

Referenzen: https://developer.apple.com/documentation/ReplayKit und
https://support.apple.com/guide/iphone/record-and-transcribe-a-call-iph57c6590e9/ios

## Paralleler Desktop-Designauftrag

- [x] Aufgabenorientierte Navigation und Startbereich, WAI, kleine Marke.
- [x] Schmale Arbeitsfenster, progressive Offenlegung bei Vorlesen,
  Aufnahme-Konfiguration in bestehenden Einstellungen.
- [x] Sechs Browser-Tests, TypeScript, ESLint, Frontend-Build, Screenshots.
- [ ] Native Windows-/Mac-Integrationstests nach Anschluss der Geräteverbindung.

Mobile Medienpipeline umgesetzt: getrenntes Originalarchiv, 30-Sekunden-STT-
Fortschritte, fortsetzbare lokale Auswertung mit Qwen 2.5 1.5B, Importansicht,
Ergebnisse mit Transkriptwechsel, Kopieren und TXT/HTML/SRT/JSON/Original-Teilen.
70 Swift-Kerntests bestehen. Synthetisches 45-Sekunden-Video nach App-Beendigung
bei Sekunde 30 ohne doppelte Segmente abgeschlossen; Original-Hash unverändert.
Die Auswertung ist ein prüfbarer Entwurf: echte Modellläufe enthielten STT-Fehler
und unvollständige Kategorien. Keine Behauptung fehlerfreier Protokolle.

Redeanteile sind im Kern nur für belegte Sprecherzuordnungen verfügbar; der
aktuelle Import liefert keine Diarisierung, daher zeigt die UI keine erfundenen
Prozente. Geräteverbindung, Synchronisierung und entfernte Mikrofone bleiben offen.

## Verbindliche UI-Prüfung: Hell und Dunkel

Benutzerergänzung 06.09.2026: Jede neue oder geänderte Oberfläche in beiden
Darstellungen prüfen; Watch zusätzlich in ihrer dunklen Systemdarstellung.
WAI-Tokens beibehalten. Aktive/deaktivierte Bedienelemente, Eingabefelder,
Menüs, Fehlermeldungen und gespeicherte Ergebnisse müssen lesbar bleiben.
Desktop und Mobile-Aufzeichnungen in Hell/Dunkel nachgewiesen; iPhone jeweils
sechs Oberflächentests erfolgreich, finale Kontraständerungen separat nachgeprüft.
Neue Gerätekopplung benötigt vor Abschluss eigene Nachweise in beiden Modi.
