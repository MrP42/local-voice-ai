# Vorschau Sprachfassungen (lva#344), Runde 3

Statische Konzeptvorschau, `vorschau.html?s=1..11&t=hell|dunkel`. Spezifikation:
`docs/superpowers/specs/2026-11-06-sprachfassungen.md` (Modell Schema 2, Migration, Sync, Konflikte).

| Bild | Inhalt |
|---|---|
| sf-01 | Bibliothek (6 Bücher): Sprachkürzel, Hinweis auf ein sicheres Paar |
| sf-02 | Prüfen-Dialog: sicheres Paar (Werkschlüssel) an, unsicheres Paar aus |
| sf-03 | Import: „Als englische Sprachfassung hinzufügen“ |
| sf-04 | Buchseite: Sprachfassungen, „Zuletzt“, Menü am ⋮ (mobil als Blatt), getrennte Lösch-Aktionen |
| sf-05 | Player: Sprachknopf mit Dropup, KI-Übersetzung gekennzeichnet |
| sf-06 | Wechsel kapitelgenau (Zuordnung bestätigt), Toast mit Rückgängig |
| sf-07 | Wechsel ungefähr: Quellkapitel liegt in unbestätigter Gruppe (16 → 17, vgl. sf-08) |
| sf-08 | Dialog „Kapitel zuordnen“: vollständige 18 ↔ 21-Zuordnung in Gruppen (1:1, 1:n), zwei Gruppen offen, „Alle bestätigen“ gesperrt |
| sf-09 | iPhone: ein Blatt „Sprache und Quelle“ (gleicher Zustand wie „Quelle“ aus lva#326) |
| sf-10 | iPhone-Regal mit Sprachkürzeln, Toast |
| sf-11 | Konflikt nach Abgleich: „Auf dem iPhone zuletzt English gehört“ |

`belege.json` je Bild: `umfang` (Selektor, Zahl geprüfter/verdeckter Bedienziele und Textknoten),
`bedienziele` (Element, Beschriftung, Rechteck x/y/w/h), `texte` (Element, Text, Schriftgröße, Kontrast nach
Alpha-Verrechnung aller Hintergrundebenen), `zuKlein`, `abgeschnittenHorizontal`, `kontrastUnter45`.
Ergebnis über 40 Bilder (`kennzahlen.json`): 1038 Texte, 282 Ziele, keine unter 44 px, keine horizontal
abgeschnitten, kleinster Kontrast 4,67:1 (Farben mit Alpha und Verläufen verrechnet). Coverbeschriftungen
sind einbezogen; verdeckte Elemente (hinter Dialog/Blatt) werden gezählt, nicht bewertet.
Grenze: Das ist eine statische Messung. Der vollständige Bediennachweis (Tastatur, Fokusführung,
Rückgängig, Konfliktfrage) folgt mit Interaktionstests in PR 2.
