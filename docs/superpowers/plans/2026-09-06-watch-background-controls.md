# Watch: Hintergrundaufnahme und Komplikationssteuerung

Benutzererweiterung vom 06.09.2026, ergänzend zur laufenden Geräteinstallation.
Gewünscht: vom Zifferblatt starten, pausieren, fortsetzen, stoppen, Laufzeit und
aktuellen Zustand sehen; vollständige App bei Bedarf öffnen. Keine automatische
Aufnahme ohne ausdrückliche Bedienung. Bestätigte Audiodaten dauerhaft erhalten.

## Stand und Abnahme

Die zunächst implementierten WidgetKit-Komplikationen Sprechen, Verlauf und letzte
Notiz sind Navigations-Schnellzugriffe. Sie erfüllen die Hintergrundsteuerung noch
nicht. Sie unterstützen circular, rectangular, inline und corner, Systemtönung und
dunkle Darstellung. Keine erfundene Live-Aktivität oder Transkriptvorschau.

Für den Ausbau sind gesondert zu prüfen:

- AudioRecordingIntent im Watch-App-Prozess, Mikrofonfreigabe und tatsächlich
  fortgesetzte Aufnahme bei inaktiver App, gesenktem Handgelenk und App-Wechsel.
- Apples AudioRecordingIntent-Dokumentation verlangt eine laufende Live Activity.
  Im installierten watchOS-26.2-SDK ist AudioRecordingIntent ab watchOS 11 verfügbar,
  ActivityKit als Watch-Framework hingegen nicht vorhanden; LiveActivityIntent ist
  auf watchOS ausdrücklich unavailable. Diesen Widerspruch bzw. den unterstützten
  Ausführungspfad vor einer Funktionszusage durch einen Geräteversuch klären.
- Gemeinsamer, dauerhaft gespeicherter Zustand für App und Widget; Berechtigungen
  des vorhandenen Personal Teams prüfen. Keine unechten Aktivitäten oder Workouts
  verwenden, um Hintergrundlaufzeit zu erzwingen.
- Sitzung + Befehls-ID gegen veraltete/doppelte Start-/Pause-/Stopp-Befehle. Bei
  Pause bereits gespeicherte Abschnitte behalten; beim Fortsetzen Zeit ohne Pause
  zählen; keine Neuaufnahme bei einem wiederholten Stopp.
- Native Timerdarstellung aus Zeitstempeln statt sekündlicher Reload-Schleife.
  Bei veraltetem Status Zeitpunkt und „Status aktualisieren“ anzeigen. Animation
  nur bei tatsächlicher Aktivität und innerhalb der WidgetKit-/Always-On-Grenzen.
- Lokale Transkription weiterhin auf dem iPhone; offline oder ohne verfügbare
  iPhone-Ausführung „gespeichert – Verarbeitung folgt“. Watch hat keinen LLM-Stack.
- Alle vier Komplikationsfamilien, mehrere Tippziele soweit brauchbar, Dark Mode,
  getönte Zifferblätter, Always-On und Öffnen der vollständigen App prüfen.

## Primärquellen

- https://developer.apple.com/documentation/appintents/audiorecordingintent
- https://developer.apple.com/videos/play/wwdc2024/10205/
- https://developer.apple.com/documentation/widgetkit/linking-to-specific-app-scenes-from-your-widget-or-live-activity
- https://developer.apple.com/documentation/widgetkit/animating-data-updates-in-widgets-and-live-activities

## Geräteinstallation

iPhone 15 Pro Max: aktualisierte App am 06.09.2026 installiert und erfolgreich gestartet.
Apple Watch Ultra 3: Entwicklermodus jetzt aktiviert bestätigt; gerätespezifischer
signierter Build und Installation erfolgreich. Vorübergehende CoreDevice-Verbindungsabbrüche
wurden durch erneute Verbindungsversuche überwunden. Hintergrundsteuerung bleibt separat zu implementieren und zu prüfen.
Die Installation auf echten Geräten darf nicht aus Simulator-Nachweisen abgeleitet werden.
