# P2 nach dem nativen Simulator-Prototyp

P0/P1 bleiben der aktuelle Implementierungsumfang. Vor vollständiger P1-Abnahme
sind Lock, Wrist Down und die verbleibenden Audio-/Force-Quit-Fälle im vereinbarten
Simulatorumfang zu prüfen. Physische Akku-/Funknachweise sind damit nicht ersetzbar.

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
