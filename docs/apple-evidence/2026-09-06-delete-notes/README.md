# Sprachnotizen löschen – 06.09.2026

Implementierung: `a152618`, isoliert auf `codex/watch-conversation-background`.

Papierkorb in der iPhone-Detailansicht, Löschknopf oben in der Watch-Detailansicht
und Wischaktion im Verlauf. Eine volle Wischgeste löscht nicht direkt. Beide Wege
zeigen „Sprachnotiz löschen?“ mit „Abbrechen“ und „Endgültig löschen“. Der Dialog
nennt Originalaufnahme, Transkript und Antwort sowie die Beschränkung auf dieses
Gerät. Kopien auf anderen Geräten bleiben erhalten; geräteübergreifendes Löschen
ist nicht Bestandteil dieses Schritts.

Bei Bestätigung werden zugehörige Wiedergabe, Verarbeitung und ausstehende
Systemübertragungen soweit lokal möglich gestoppt. Die Notiz wird atomar aus dem
Verlauf verschoben, ihre Inhalte entfernt und eine leere Löschmarkierung behalten.
Diese verhindert eine erneute Annahme derselben Session durch verspätete Transfers.
Nach einem Abbruch während des Löschens wird die Inhaltsbereinigung beim nächsten
Start vervollständigt. Andere Notizen bleiben erhalten. Keine automatische Löschung
und keine Löschung echter Nutzernotizen während der Tests.

## Prüfung

89 Core-Tests bestanden, einschließlich Erhalt anderer Notizen, tatsächlicher
Audioentfernung, abgewiesener verspäteter Zustellung/Antwort, Neustartbeständigkeit
und Bereinigung einer unterbrochenen Löschung. Vier Bedienungsläufe bestanden:
iPhone-Detail und Wischgeste im Dark Mode, iPhone-Detail im Light Mode und
Watch-Detail. Jeder Test prüft Abbrechen und anschließend bestätigtes Löschen.
Die UI-Tests verwenden isolierte synthetische Sprachnotizen.

Erste UI-Läufe deckten eine fehlende Dialogdarstellung, das vorzeitige Ausblenden
einer destruktiven Wischzeile und die Watch-Toolbar-Darstellung auf. Korrigiert durch
einen einheitlichen Bestätigungsdialog, eine erst beim Bestätigen destruktive Aktion
und einen direkten Watch-Knopf. Die abschließenden Wiederholungsläufe bestanden.
Signierter Geräte-Build für iPhone und eingebettete Watch erfolgreich.

Endgültige Fassung auf echtem iPhone und Watch am 06.09.2026 installiert.
Löschbestätigung im exportierten Screenshot visuell geprüft.
