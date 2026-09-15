# macOS-Systemmodelle und optionale Vorlesemodule

Umsetzung: [PR #32](https://github.com/MrP42/local-voice-ai/pull/32).
Windows-Abnahme und optionale Fish-Paketierung:
[Issue #29](https://github.com/MrP42/local-voice-ai/issues/29).

Bei einer neuen macOS-Einrichtung verwendet Vorlesen die vorhandene Systemstimme.
Weitere installierte Systemstimmen lassen sich im Stimmenfeld auswählen. Piper
und Fish sind zusätzliche Module; fehlende oder unvollständige Module dürfen
weder als verfügbare Stimme noch mit ihren spezifischen Einstellungen erscheinen.
Die bisherige unbenannte, nicht funktionsfähige Fish-Standardauswahl wird migriert.
Bereits gewählte Modelle und funktionierende eigene Installationen bleiben erhalten.

Unter Modelle steht Apple Speech bereit, wenn macOS eine lokale Sprachvariante
unterstützt. Das Betriebssystem verwaltet deren Dateien; die App bietet dafür
keinen eigenen Download oder Löschknopf. „macOS-Systemsprache“ folgt der
Systemsprache, sie bedeutet keine automatische Spracherkennung. Die erste
Erkennung benötigt gegebenenfalls die macOS-Freigabe für Spracherkennung.
Die native Anfrage erzwingt lokale Verarbeitung und fällt nicht auf einen
Spracherkennungsserver zurück.

Das Apple-LLM wird nur bei gemeldeter Verfügbarkeit angeboten und ohne bestehende
Nutzerauswahl als Standard eingetragen. Diese Foundation-Models-Anbindung erfordert
Apple Silicon, macOS 26 oder neuer und ein verfügbares Systemmodell. Auf einem
Intel-Mac bleibt für KI-Zusammenfassungen ein zusätzliches Modell erforderlich.

Piper installiert die zur Plattform passende Laufzeit mit der ersten Stimme.
Der Installer prüft Dateien und Programmstart vor dem Austausch und erhält bei
Fehlern die vorherige Installation. Ein vollständiger optionaler Fish-Installer
ist weiterhin Gegenstand von Issue #29 und gehört nicht zu diesem Stand.

Die Windows-Abnahme soll insbesondere Erstinstallation ohne Module, fehlende
DLLs, alte Teilinstallationen, Abbruch/Neustart und parallele Downloads sowie
Entfernen einer ausgewählten Stimme prüfen. Die macOS-System-APIs bleiben
plattformabhängig; der Windows-Sprachpfad wird dadurch nicht ersetzt.

## Einrichtung von KI-Textfunktionen

Übersetzen, Zusammenfassen, Textaufbereitung und Protokoll unterscheiden zwischen
fehlender Einrichtung und einem Fehler während der Verarbeitung. Ohne gewähltes
Sprachmodell zeigen sie einen neutralen Hinweis mit „Modell auswählen oder
installieren“ und einer optionalen Anbieter-Verknüpfung. Installierbare Modelle
und verfügbare Systemmodelle stehen unter Modelle; externe Verbindungen bleiben
in den bestehenden KI-Einstellungen.

Eine Auswahl benötigt keinen bereits gestarteten Modellprozess. Leerlauf und
fehlender Download sind verschiedene Zustände. Die Fußleiste verwendet für eine
noch ausstehende Modellauswahl eine neutrale Anzeige. Vorwärmen ist nur bei einer
passenden Ollama-Verbindung verfügbar. Fehler während einer tatsächlichen
Verarbeitung oder beim Speichern bleiben sichtbar.

Der Einrichtungssprung aus dem Vorlesen sichert den aktuellen Text vor dem
Seitenwechsel. Scheitert diese Speicherung, bleibt der Text zur Bearbeitung offen.
Deutsch und Englisch sowie Hell- und Dunkelmodus sind abgedeckt.
