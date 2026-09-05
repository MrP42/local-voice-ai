# P2 nach dem nativen Simulator-Prototyp

**Fortschreibung:** P2 wurde anschließend vom Benutzer beauftragt. Implementierung
und noch offene Abnahme stehen im [aktuellen P2-Bericht](p2/README.md). Der folgende
Text bewahrt den Ausgangsplan nach P1.

P0/P1 bleiben der aktuelle Implementierungsumfang. Der abschließende Dauerlauf
auf `aad2e0f` ist mit 100/100 lokal beantworteten und quittierten Aufträgen bestanden;
31 Kerntests, beide nativen Bedienungstests und frischer Simulator-Build bestehen.
Damit ist P1 im vereinbarten Simulatorumfang abgeschlossen, bei weiterhin
nicht erreichten Latenzzielen. Der externe Claude-Review
ist inzwischen durchgeführt und die bestätigten P0/P1-Befunde sind bearbeitet. Lock, Wrist Down und benutzerseitiges
Beenden sind inzwischen als Deferred-Fälle im Simulator nachgewiesen. Physische
Audio-Routen und Anrufunterbrechungen sind nicht aus dem expliziten Stop-Test ableitbar. Physische Akku-/Funknachweise sind damit nicht ersetzbar.

1. **Sprachqualität und Laufzeit:** Base-/Small-Vergleich aus den fünf deutschen
   Qualitätsfällen auswerten; eigene kurze Antwortfälle mit Kriterien für Texttreue
   ergänzen. Antwortmodell darf keine ausgeführten externen Aktionen behaupten.
   Getrennte Warm-/Kaltstartmessungen und Ende-zu-Ende-p95 statt Addition von Quantilen.
2. **Jobsteuerung:** VoiceModel in Aufnahme, Transport und persistente Jobsteuerung
   trennen. Prozessabbruch an jeder Commit-/Quittungsgrenze testen. Begrenzte Wiederholung,
   Abbruch während Inferenz und Foreground-Ablauf explizit modellieren. Bestehende stabile
   Antwortquittungen und dauerhafte Teiltransfer-Wiederaufnahme beibehalten.
3. **Speicher:** volle Platte während Schreiben, beschädigte Metadaten und teilweise
   Dateien gezielt prüfen. Wiederherstellungsoberfläche für unbestätigte Recorderdateien,
   additive Migration und Budget für Transfer-/Staging-Kopien. Aufbewahrung und Löschen
   erst nach nachgewiesenem Quittungsprotokoll ergänzen.
4. **Lifecycle:** verbleibende virtuelle UI-Fälle automatisieren. Hardwareabnahme erst
   als eigener beauftragter Schritt: Data Protection, Swipe-Force-Quit, reale Audioausgabe,
   Funk und Wrist Down. Keine fingierte Workout-Sitzung für längere Laufzeit.
5. **Provider:** Apple Speech/Foundation Models weiterhin zur Laufzeit prüfen. CPU-Fallback
   auf iPhone belassen; Modellinstallation und Speicherbedarf sichtbar machen. Modellhashes
   vor Verwendung prüfen. Keine Modellgewichte oder Desktop-Engines auf der Watch.
6. **Energie:** bei späterer Hardwareabnahme drei vergleichbare Läufe mit/ohne App,
   8 Stunden/30 Turns, Watch und iPhone getrennt. Bis dahin keine Akku-Prozentangaben.

Keine Desktop-Neugestaltung, Android-Portierung, App-Store-Veröffentlichung oder Push
nach main innerhalb dieses Auftrags. Suche, Sammlung und weitere Produktfunktionen
folgen erst nach belastbarer Machbarkeitsentscheidung.

## Ergänzungen aus Review und Wiederholung

- Verwaiste Staging-Verzeichnisse sicher zuordnen und bereinigen, ohne unbestätigte
  Recorderdateien oder bestätigte Originale still zu löschen.
- Korrupten Verlauf sichtbar isolieren und übrige Verarbeitung ermöglichen; keinen
  `compactMap`-Fallback einführen, der bestätigte Daten aus der Anzeige entfernt.
- Native Speech-/Foundation-Models-Cancellation und Segmentgrenzen auf einem
  tatsächlich unterstützten Ausführungsziel prüfen. Simulator-CPU-Ergebnisse sind
  kein Nachweis für Apples Modelle.
- Leere/geräuschhaltige Aufnahmen und frei erfundene Transkripte/Antworten in das
  Qualitätsset aufnehmen. Base/Small unterscheiden sich in Qualität und Laufzeit;
  kein getesteter Kandidat erfüllt bislang das ursprüngliche Latenzziel.
- Transport-Laufzeittests beibehalten: statische Reviews hatten die nun gemessene
  zusätzliche Transferlatenz nicht erkannt. Interaktiven Pfad und Dateifallback
  mit begrenztem Rückstand getrennt messen.
