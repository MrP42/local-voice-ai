# Nach P2 – verbleibende Entscheidungen und Abnahmen

Kein neuer Implementierungsauftrag und keine Veröffentlichung durch dieses Dokument.

1. **Antwortqualität vor Funktionsausbau.** Das aktuelle 0,5B-Modell ist bei einfachen
   Fakten/Rechenfragen nicht zuverlässig. Größere lokale Antwortmodelle auf einer
   geeigneten iPhone-Laufzeit gegen dieselben und neue unabhängige Fälle vergleichen.
   Keine Benchmark-Fragen fest verdrahten. Aktionsbehauptungen zusätzlich mit
   mehrsprachigen, negierten und indirekten Formulierungen prüfen; der heutige
   deutsche Wortfilter ist eine begrenzte Absicherung.
2. **Latenz auf dem Zieltelefon entscheiden.** Apple Speech/Foundation Models und
   unterstützte lokale Alternativen auf einem tatsächlichen iPhone messen. Base
   bleibt im Intel-Simulator der schnellere getestete STT-Kandidat; Small ist kein
   Ersatz für das verfehlte Echtzeitziel. Neue Optimierungen anhand echter E2E-Zeiten
   bewerten, einschließlich Modellladen, Rücktransport und TTS-Start.
3. **Native Simulator-Bedienungsfälle beibehalten.** Lock, Wrist Down und
   benutzerseitiges Wegwischen wurden am 06.09. frisch auf P2 bestanden. Künftige
   Regressionen benötigen einen entsperrten Host und eine visuelle Prüfung der
   richtigen App-Karte vor dem Wegwischen.
4. **Hardwareabnahme separat beauftragen.** Watch Ultra 3 / iPhone 15 Pro Max:
   Audio-Routen und Anrufe, gesperrte Dateien, echte Funkunterbrechungen und
   Force-Quit-Zustellung testen. Keine Workout-Sitzung zur Laufzeitverlängerung.
5. **Energie messen.** Drei vergleichbare 8-h/30-Turn-Läufe mit/ohne App, Watch und
   Telefon getrennt, dokumentierte Anfangsladung und Umgebungsbedingungen. Bis dahin
   keine Aussage über Akku-Prozentverbrauch oder Alltagstauglichkeit.
6. **Aufbewahrung erst mit konkreter Nutzerentscheidung.** Bestätigte Originale
   bleiben aktuell bestehen. Eine Lösch-/Exportpolitik muss Quittungen, Backups,
   Wiederherstellung und volle Speicher klar abdecken. Keine automatische Räumung
   zur Verschönerung von Dauertests.

Desktop-Tauri, Android, Suche/Sammlungen, App-Store-Veröffentlichung und Push nach
main bleiben außerhalb dieses Apple-Prototyps.
