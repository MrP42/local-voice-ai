# Gespräch nach Geräusch oder leerer Transkription – 06.09.2026

## Korrektur

Drei Fehlerpfade korrigiert: Der lokale 8-Sekunden-Ruheabschluss pausiert nicht mehr
das Gespräch. Ein leeres Apple-SpeechTranscriber-Ergebnis ist nun `noSpeech` statt
ein allgemeiner Fehler mit Wiederholungsversuchen. Bei einem Gesprächsbeitrag ohne
Sprache erzeugt die Verarbeitung ein dauerhaft gespeichertes, als Nicht-KI markiertes
Systemergebnis. Dieses nutzt auch zur Watch die vorhandene quittierte, deduplizierte
Antwortzustellung. Der Gesprächsmodus liest den Hinweis nicht vor und hört nach
350 ms wieder zu. Die Originalaufnahme bleibt erhalten; der leere Beitrag gelangt
nicht zum LLM und nicht in dessen Gesprächskontext. Stopp, fehlende Mikrofonfreigabe,
Speicherfehler und Verlassen der App bleiben echte Pausengründe.

## Mikrofon

Drei auf dem jeweiligen Gerät gespeicherte Empfindlichkeiten: Unempfindlich,
Ausgewogen (Standard), Empfindlich. Dazu standardmäßig aktive Anpassung an den
Umgebungspegel. Die ersten 0,6 Sekunden dienen der Einmessung; unterhalb der
Sprachschwelle folgt der Pegel langsam der Umgebung. Bei vermuteter Sprache wird
er nicht angehoben. Dies ist eine Pegelschwelle zur Sprechpausenerkennung, keine
semantische VAD und keine Filterung der gespeicherten Audiodatei. Gleichmäßige
Geräusche werden seltener als Sprache gewertet; wechselnde Geräusche und andere
Stimmen können weiterhin auslösen. Einstellungen gelten ab dem nächsten Beitrag.

Erreichbar unter Einstellungen → Mikrofon & Gespräch und in den Gesprächsoptionen
auf iPhone/Watch. Die Änderung der Schwelle verändert nicht die Aufnahmelautstärke.

## Nachweise

87 Core-Tests bestanden, darunter drei neue Tests für gleichmäßiges Raumgeräusch,
Empfindlichkeitsunterschied und dauerhaftes Nicht-KI-Ergebnis samt Watch-Duplikaten
und unverändertem Original. Zuerst fehlgeschlagener Testlauf wegen noch nicht
implementierter Schnittstellen, danach erfolgreich. Zwei iPhone-Bedienungstests
bestanden: leere Transkription → Mikrofon wieder aktiv → Stopp (14,540 s), normale
Antwort → Zuhören → beim Verlassen pausieren (18,253 s). Testzeiten sind komplette
UI-Laufzeiten, keine STT-Latenzen. Die Provider sind in den UI-Proben deterministisch;
es handelt sich nicht um einen akustischen Test mit echten Umgebungsgeräuschen.

Ein Screenshot zeigt den wieder aktiven Gesprächsmodus im hellen Design vor der
abschließenden expliziten Beschriftung der Mikrofonempfindlichkeit. Bestehende
WAI-Farben und dynamische Systemfarben bleiben erhalten.

Noch offen: akustische Abnahme unter den realen Hintergrundgeräuschen des Benutzers,
Fehlauslösungsrate, leise Sprache direkt bei Aufnahmestart sowie Langzeit-/Akkumessung.
Keine Behauptung vollständiger Geräuschunterdrückung. Bestätigte Originale werden
nicht automatisch gelöscht; längere reine Geräuschgespräche können Speicher belegen.

Abschließender Dark-Mode-Bedienungstest ebenfalls bestanden; Screenshot visuell geprüft.
Signierter Geräte-Build für iPhone samt Watch erfolgreich. Beide Apps installiert;
die akustische Abnahme ist damit noch nicht ersetzt. Implementierung: `68e4dc1`.
