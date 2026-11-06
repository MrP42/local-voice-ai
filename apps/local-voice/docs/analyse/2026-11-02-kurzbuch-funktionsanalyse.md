# Funktionsanalyse Kernaussagen-Zusammenfassungen (Vorbild Blinkist) – lva#205

Stand 02.11.2026. Vorgehen nach dem Skill `reverse-engineer-anything` (Fragenliste,
Belegbuch, Beobachtung / Schluss / Unbekannt getrennt). Die REA-Werkzeuge selbst
waren in dieser Sitzung nicht lauffähig (`rea doctor`: Node 25 nicht unterstützt,
Registrierung veraltet); beobachtet wurde deshalb die öffentliche Buchseite im
eingebauten Browser, ohne Anmeldung und ohne App.

Übernommen werden **nur Funktionen und Struktur**. Keine Texte, Marken, Bilder oder
Gestaltung; die Umsetzung folgt dem eigenen Fluent-Stil von Local Voice AI.

## Fragen

1. Wie ist eine Zusammenfassung gegliedert? – beantwortet (B1–B4)
2. Welche Metadaten sieht man vor dem Lesen? – beantwortet (B5)
3. Wie verbinden sich Lesen und Hören? – teilweise (B6), App nicht untersucht
4. Wie entsteht der Inhalt? – unbekannt (redaktionell, nicht beobachtbar)

## Belegbuch (Beobachtungen)

| ID | Quelle | Beobachtung |
|----|--------|-------------|
| B1 | Buchseite „Rich Dad, Poor Dad“, Abschnitt Inhaltsübersicht | Nummerierte Liste von 10 Einträgen; der letzte heißt „Zusammenfassung“ (Fazit). |
| B2 | dieselbe Liste | Jeder Titel hat die Form „Schlagwort: ganzer Aussagesatz“. |
| B3 | erste Kernaussage, ausgeklappt | Kennzeichnung „Kernaussage 1 von 10“, Überschrift, etwa 350 Wörter Fließtext, Einstieg mit einer Frage an den Leser, Knopf „Nächste Kernaussage“. |
| B4 | Seitenende | Abschnitte „Worum geht es in …?“ (ein Absatz), „Wer … lesen sollte“ (drei Punkte, je „Jeder, der …“), „Über den Autor“. |
| B5 | Seitenkopf | Kategorie, Titel, Autor, Untertitel des Buchs, Bewertung, Dauer („21 Min.“), Zahl der Kernaussagen, Hinweis „Zum Lesen & Anhören“, Kurzbeschreibung in einem Satz, Themen. |
| B6 | Seitenkopf | „Einleitung anhören“ mit Länge (02:21): Audio je Abschnitt, nicht nur fürs Ganze. |

## Schlüsse

- S1 (aus B1–B3): Die Einheit ist die einzelne Kernaussage. Lesen, Hören und
  Fortschritt laufen je Kernaussage; das Fazit ist eine eigene Einheit.
- S2 (aus B5, B6): Dauer ist die Hauptkennzahl vor dem Start; sie gilt für Lesen
  und Hören gleichermaßen.
- S3 (aus B4): „Für wen“ und „Worum es geht“ helfen bei der Entscheidung, ob man
  das Werk lesen will; sie stehen vor den Kernaussagen.
- S4: Über den Autor braucht externe Daten; für lokale Bücher entfällt es oder
  kommt aus den Bibliotheksmetadaten.

## Übertragung auf Local Voice AI

| Funktion | Umsetzung |
|----------|-----------|
| Kernaussagen mit Schlagwort-Titel | Vorlage `builtin:kurzbuch`, Überschrift `## <Schlagwort>: <Satz>` |
| Fazit als eigene Einheit | letzter Abschnitt `## Fazit` |
| Worum es geht, Für wen | Abschnitte vor den Kernaussagen |
| Dauer vorab | aus der Wortzahl: Lesen 200 Wörter/Min, Hören nach Vorlesetempo |
| Hören je Kernaussage | Vorlesen ab der Überschrift der Kernaussage (vorhandener Reader) |
| Fortschritt | vorhandener Hörfortschritt der KI-Dokumente, je Kernaussage abgeleitet |
| Hörbücher | Quelle ist das Transkript (Textfassung `origin = transcript`) |

## Unbekannt

- Wie die Inhalte entstehen (Redaktion, Werkzeuge) – nicht beobachtbar, für uns
  unerheblich; wir erzeugen lokal per Sprachmodell aus dem Buchtext.
- Verhalten der App (Offline, Merken, Markieren) – nicht untersucht.
