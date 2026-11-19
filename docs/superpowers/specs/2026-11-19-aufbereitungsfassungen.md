# Aufbereitungsfassungen — A0, lva#659

Stand: 19.11.2026. Umfang: Datenmodell, reine Berechnung, dokumentnaher Speicher
und Gegenproben; keine UI, Modellaufrufe, Commands oder Änderung von `lib.rs`.

## Abgrenzung

Sprachfassungen (`library/editions.rs`, #344) enthalten denselben Inhalt in einer
anderen Sprache. Eine Aufbereitung bearbeitet eine konkrete Sprachfassung oder
Seite bzw. ein Gespräch. Sie ist keine Edition und steht nicht in der Sprachauswahl.
`dokument.fassung_id` bezeichnet gegebenenfalls Edition oder Gesprächsvariante.

Die Seitenversionierung (#546, Entwurf #567) liefert die eingefrorene Basisversion
mit SHA-256 und CAS-Prüfung. Dieses Modul ersetzt weder Versionierung noch CAS.
Gespräche werden in A4 an `meetings/variants` angebunden; es entsteht kein zweiter
Variantenbestand. Jeder Speicher gehört genau einem Dokument bzw. einer konkreten
Fassung. Der Aufrufer bestimmt dessen vertrauenswürdigen Ordner.

## Modell

`Aufbereitung`: ULID `id`, `dokument { art: buch|seite|gespraech, id, fassung_id? }`,
`basis { sha256, laenge }`, `vorgaenger_aktiv?`, `schritte`, `modell?`, `status`,
`fortschritt { abschnitte_fertig, abschnitte_gesamt }`, `aenderungen`,
`erstellt`, `geaendert`. Zeiten sind Unix-Millisekunden (`u64`), Länge ist die
Byte-Anzahl (`u64`); SHA-256 ist hexadezimal in Kleinbuchstaben.

Schritte: `struktur`, `format`, `sprache`, `glaetten`. Modellmetadaten enthalten nur
`funktion: Schritt` und `ziel_art: lokal|cloud`; keinen Prompt und keinen Inhalt.

`Aenderung`: `id`, `art: Regel(String)|sprache|glaetten`, `bereich: [start, ende]`
als JSON-Paar mit halboffener Bedeutung, `alt`, `neu`, `grund_code`, `zaehler?`,
`gruppe?`, `status: offen|angenommen|abgelehnt`. Regelname und Grundcode sind
stabile technische Kennungen, keine Freitexterklärungen mit Dokumentinhalt.
Änderungs-IDs sind innerhalb einer Aufbereitung eindeutig.

**Entscheidung: Byte-Bereiche.** `[start, ende)` liegt auf UTF-8-Zeichengrenzen in
der eingefrorenen Basis. SHA-256 und Länge identifizieren diese Basis; Versätze
verschieben sich bei Annahme nicht. Absatzanker würden nur eine veränderliche
Basis adressieren und zusätzliche Auflösungsregeln verlangen. Bei verändertem
Text wird ohnehin nicht angewendet, sondern neu berechnet.

## Regeln 1–9

1. **Original:** `original.txt` wird beim ersten Aufbereiten einmal eingefroren.
   Gleiches erneutes Schreiben ist erfolgreich, anderer Inhalt ergibt
   `OriginalFest`. Aufruf vor der ersten Speicherung ist Pflicht.
2. **Entwurf:** Erst Übernehmen liefert den aktiven Text. `ergebnis_text` setzt
   ausschließlich angenommene Änderungen auf die Basis auf, nach Byte-Bereich
   sortiert. Das Modell liefert keinen fertigen Ergebnistext. `uebernehmen`
   akzeptiert ausschließlich `entwurf`; alle anderen Status ergeben `Status`
   ohne Mutation. Danach prüft es den aktuellen Text vor der Auswertung und
   aktiviert nur bei Erfolg. Auch eine wieder passende Basis erlaubt kein erneutes
   Übernehmen einer veralteten oder verworfenen Fassung.
3. **Teilannahme:** Alle Vorschläge werden geprüft, auch offene und abgelehnte.
   Bereiche dürfen sich nicht überlappen; doppelte Einfügungen am selben Punkt
   und Einfügungen am Beginn eines anderen Bereichs gelten als mehrdeutig und
   werden abgelehnt. Benachbarte Ersetzungen sind erlaubt. `alt` muss exakt passen.
   Mitglieder derselben Gruppe müssen denselben Status haben; der Aufrufer setzt
   sie gemeinsam, Zwischenstände werden weder berechnet noch gespeichert.
4. **Rückgängig:** `rueckgaengig` liefert `Some(Some(vorgaenger_id))` oder
   `Some(None)` für Original. Äußeres `None` bedeutet unbekannte aktive ID.
   Der Aufrufer aktiviert diese Referenz; die Kette wird nicht verändert.
5. **Konflikt:** SHA-256 **und** Byte-Länge müssen passen. Bei Abweichung setzt
   `uebernehmen` den Status auf `veraltet`, aktualisiert `geaendert` und liefert
   ausschließlich einen Fehler. Der Aufrufer veröffentlicht keinen Text und bietet
   Neuberechnung an. Veröffentlichung und CAS bleiben Aufgabe der Anbindung.
6. **Schutz:** `{{S. n}}` mit nichtnegativer Dezimalzahl, nichtleere einzeilige
   `<…>`-Sprechermarken und `[…]`-Tags bleiben unverändert. Zeitanker werden als
   `MM:SS` oder `HH:MM:SS`, optional `.mmm` bzw. `,mmm`, erkannt (je zwei Ziffern
   pro Zeitteil). Eckige Klammern sind bereits vollständig geschützt. Konservativ
   gelten alle passenden Winkel-/Klammermarken als geschützt. Ersetzen/Löschen
   eines Teils oder Einfügen im Inneren ist ungültig; unmittelbar davor/danach
   ist erlaubt. Neue Marken im Ersatztext sind erlaubt.
7. **Speicher:** `<dokumentordner>/aufbereitung/`, zum Beispiel unter
   `books/<id>/` oder `projects/<page>/`. Original und JSON werden über
   `crate::commands::pages::write_atomic` geschrieben. IDs werden vor Pfadbildung
   ausschließlich auf 26 ASCII-Zeichen `[0-9A-Z]` geprüft. Neue IDs kommen aus
   `ulid::Ulid::new()`. Kein Dokumentpfad wird aus einer Fassungs-ID gebildet.
8. **Aufbewahrung:** `verworfen` und `veraltet` sind ab 30 vollen Tagen seit
   `geaendert` löschbar; Zukunftszeiten laufen nicht ab. Andere Status bleiben.
   `aufraeumen` entfernt abgelaufene Dateien, `alles_loeschen` entfernt den gesamten
   Aufbereitungsordner einschließlich Original. Das Löschen eines Dokuments muss
   diese Operation auslösen. Explizites Einzellöschen entfernt nur die JSON-Datei.
9. **Vergleich:** Wortdiff bleibt beim bestehenden `variants/diff.ts` (Anbindung
   später). `struktur_vergleich(vorher, nachher)` ist rein und liefert beide
   Strukturen: ATX-Überschriften (`#` bis `######` mit Leerzeichen), Seitenmarken,
   Absätze und zusammenhängende Markdown-Tabellenzeilen mit äußeren `|`.
   Leerzeilen trennen Absätze; Tabellen/Überschriften sind eigene Elemente.
   Diese begrenzte Markdown-Erkennung ist kein vollständiger Markdown-Parser.

## Statusübergänge

Die Tabelle ist der Vertrag für spätere Aufrufer; A0 implementiert keine UI und
keinen allgemeinen Statusautomaten. Nur Übernehmen setzt selbst einen Status.

| Ausgang | Ereignis | Ziel | Wirkung |
| --- | --- | --- | --- |
| neu | Verarbeitung starten | laeuft | Basis einfrieren, Fortschritt 0 |
| laeuft | Abschnitt fertig | laeuft | abschnitte_fertig erhöhen |
| laeuft | fertig | entwurf | Vorschläge prüfbar |
| laeuft | abbrechen | abgebrochen | Fortschritt erhalten |
| abgebrochen | fortsetzen | laeuft | ab abschnitte_fertig weiter |
| entwurf | Teilannahme/-ablehnung | entwurf | aktiver Text bleibt |
| entwurf | übernehmen, Basis passt | aktiv | deterministisches Ergebnis |
| entwurf/abgebrochen/laeuft | Basis verändert | veraltet | nichts anwenden |
| entwurf/abgebrochen | verwerfen | verworfen | Aufbewahrungsfrist beginnt |
| aktiv | Nachfolger übernehmen | abgeloest | Kette erhalten |
| aktiv | rückgängig | abgeloest | Vorgänger aktiv bzw. Original |
| abgeloest | Rückkehr durch Rückgängig | aktiv | vorhandene Fassung aktivieren |
| verworfen/veraltet | 30 Tage erreicht | gelöscht | JSON entfernen |

`geaendert` muss beim Verwerfen/Veralten auf die Ereigniszeit gesetzt werden;
Metadaten verlangen `erstellt <= geaendert` und `fertig <= gesamt`.

## Dateiformat und Schema

Eine Datei `<id>.json` enthält
`{ "schema_version": 1, "basis_text": "…", "aufbereitung": { … } }`.
Der Basis-Snapshot im Umschlag erlaubt vollständige Prüfung beim Laden ohne
veränderlichen Dokumenttext und ohne Abhängigkeit von anderen JSON-Dateien.
Der Hash im Modell wird gegen diesen Snapshot geprüft. Er ist kein Authentizitäts-
oder Manipulationsschutz gegen jemanden, der sämtliche lokalen Dateien ändern kann.

`speichern` verwendet den vorhandenen Snapshot derselben ID, andernfalls das
Ergebnis von `vorgaenger_aktiv`, andernfalls Original. Für externe Seitenversionen
steht zusätzlich `speichern_mit_basis(&Aufbereitung, &str)` bereit: der Aufrufer
liefert die konkrete Basis nach seiner CAS-Prüfung. Original muss schon existieren.
Ohne diesen Zusatz wäre die verlangte Signatur `speichern(&Aufbereitung)` für
eine externe Basis nicht ausreichend: Ein Hash enthält den Text nicht.

Beide Speicherwege prüfen vorhandene IDs gemeinsam: Dokument (Art, ID und
Fassungs-ID), Basis (Hash und Länge), Vorgänger, Basis-Snapshot und alle übrigen
unveränderlichen Felder einschließlich Vorschlägen müssen erhalten bleiben.
Nur Status, Änderungsstatus, Fortschritt und `geaendert` dürfen sich ändern.
Eine abweichende Identität oder Basis ergibt `Basis`, ohne die Datei zu ändern.

Nur Version 1 wird gelesen; ältere/neuere Versionen, fehlende Schema-Angabe und
fremde Strukturen ergeben `Schema`. Unbekannte Felder sind toleriert. Defektes
JSON, leere oder unlesbare Dateien ergeben `Lesen`. Semantisch ungültige Snapshots
oder Änderungen ergeben `Basis` bzw. `Aenderungen`. Die Datei-ID muss mit der
angeforderten ID übereinstimmen. `liste` sortiert nach ID und liefert Fehler für
ungültige JSON-Einträge, statt diese still zu verschlucken. Andere Dateiendungen
werden ignoriert, damit temporäre Dateien nicht als Fassungen gelten.

`original.lock` schützt konkurrierende Erstschreibvorgänge über `create_new`.
Ein konkurrierender Aufruf erhält `Schreiben` und kann wiederholt werden. Nach
normalem Erfolg/Fehler wird die Sperre entfernt. Ein gemeinsamer statischer Mutex
serialisiert prozessweit alle Schreib- und Löschwege einschließlich Original,
beider Speicherwege, Einzel-/Dokumentlöschung und Bereinigung; auch `liste` liest
unter dieser Sperre. Ein vergifteter Mutex wird mit `into_inner` weiterverwendet.
Die Bereinigung liest jede noch vorhandene Kandidatendatei unter der Sperre erneut
und prüft unmittelbar vor dem Löschen deren aktuellen Status und Änderungszeit.
Eine inzwischen aktualisierte Fassung wird nicht anhand alter Metadaten gelöscht.
Diese Serialisierung gilt innerhalb eines Prozesses; mehrere Prozesse muss der
Aufrufer weiterhin koordinieren.

## Datenschutz

Original, Basis-Snapshot, `alt` und `neu` enthalten lokale Dokumentinhalte und
unterliegen derselben Zugriffskontrolle wie das Dokument. Keine Logs. Fehler
enthalten nur feste Kategorien, nie Text, Pfade, IDs, JSON-Parsermeldungen oder
weitergereichte Betriebssystemfehler. Modellmetadaten enthalten nur die Art.
Cloud-Übertragung und Zustimmung dazu sind nicht Bestandteil von A0.
Debug-Ausgaben von Änderungen und Aufbereitungen sowie des Datei-Umschlags geben
keine Dokumentinhalte aus; Textfelder erscheinen nur als Länge bzw. Anzahl.

## Gegenproben und offene Punkte

Deterministische Tests decken reine Berechnung und `tempfile`-Speicher ab.
Negative Fälle stehen jeweils neben positiven Kontrollen: geänderter Hash bei
gleicher Länge, unzulässige Bereiche/UTF-8-Grenzen/Marken, falsches `alt`, gemischte
Gruppen, ungültige IDs/Schemata und beschädigte JSON-Dateien müssen fehlschlagen.
Ohne die jeweilige Validierung würden diese Fehlererwartungen rot. Permutationen
prüfen Teilannahme und Eingabereihenfolge. Hinzu kommen Roundtrip/Fortsetzen,
Rückgängig ohne Kettenmutation, Aufbewahrungsgrenzen und Schreibfehler ohne
Beschädigung vorhandener Daten oder verbleibende temporäre Datei.
Weitere Gegenproben verhindern Übernehmen aus anderen Status trotz wieder passender
Basis und Überschreiben einer vorhandenen ID mit anderem Dokument, anderer Basis
oder anderen Vorschlägen. Abgelehnte Schreibversuche lassen die Datei unverändert.
Eine vorab gelesene, abgelaufene Kandidatenliste wird nach Aktualisierung mit
`speichern` an `aufraeumen_mit` übergeben: Die erneute Prüfung erhält die Fassung,
ohne Schlafen oder zeitabhängige Nebenläufigkeit. Debug-Gegenproben prüfen, dass
der Marker `GEHEIMTEXT` aus Basis-Snapshot, `alt` und `neu` nicht ausgegeben wird.

Offen für die Anbindung: Modulzeile in `lib.rs`, konkrete Dokument-/Editionspfade,
CAS bei Veröffentlichung, atomare Aktivierung der Kette, UI/Wortdiff, A4-Anbindung
und gegebenenfalls weitere belegte Zeitanker-/Markdown-Formate. Nach Prozessabbruch
kann `original.lock` liegen bleiben; kontrollierte Wiederherstellung muss bei
gesichertem Prozessstillstand erfolgen. Rückgängig auf explizit gelöschte Vorgänger
ist nicht möglich; reguläre aktive/abgelöste Vorgänger verfallen nicht automatisch.
Symlink-/Junction-Manipulation des vertrauenswürdigen Dokumentordners durch einen
lokalen Angreifer ist nicht durch die ID-Prüfung abgedeckt.

Die Implementierung wurde ausschließlich aus den eingebetteten Quellen erstellt.
`cargo fmt`, `clippy` und Tests sowie das Verhalten des vorhandenen atomaren
Schreibers müssen anschließend im vollständigen Crate geprüft werden.
