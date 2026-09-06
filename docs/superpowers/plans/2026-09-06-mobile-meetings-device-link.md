# Mobile Gespräche, lokale Auswertung und gekoppelte Geräte

Explizite Erweiterung des Benutzerauftrags vom 06.09.2026. Der Apple-Worktree bleibt
isoliert; erforderliche Desktop-Integrationen werden hier entwickelt, nicht im
ursprünglichen Checkout mit dessen lokalen Änderungen. Kein Push/Release.

## Beauftragter Umfang

- [ ] Audio-/Videoimport auf iPhone, dauerhaftes Original und fortsetzbares lokales STT.
- [ ] Strukturierte lokale Protokolle gemäß Desktop: Zusammenfassung, Kontext,
  Entscheidungen, Aufgaben/Zuständige/Termine, nächste Schritte, Empfehlungen,
  offene Fragen. Originaltranskript bleibt daneben zugänglich.
- [ ] Redeanteile aus belegten Segment-/Kanalzuordnungen; keine vom LLM erfundenen
  Prozentwerte. Gemischter Ton ist nicht automatisch ein einzelner Sprecher.
- [ ] Kopieren, Teilen, Export von Transkript und Auswertung.
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

Mobile Kernbausteine bereits testgetrieben ergänzt: getrenntes Originalarchiv,
fortsetzbare Segmentfortschritte, Desktop-Protokollschema, belegbare Redeanteile,
SRT/HTML-Export. 65 Swift-Kerntests bestehen. Noch keine vollständige Import-UI,
Medienpipeline oder Geräteverbindung daraus ableiten.
