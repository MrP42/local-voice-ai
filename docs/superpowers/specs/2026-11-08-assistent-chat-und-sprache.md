# Spezifikation: Assistent – Chat und Sprache geräteübergreifend (Epic)

- Stand: 08.11.2026, Entwurf zur Codex-Abnahme
- Auftrag: Patrick 08.11.2026, Thread „Chat und Voice-Agent plattformübergreifend vereinen“
- Architektur: wai-sdk ADR-0028 (Entscheidungen E1–E9, offene Fragen F1–F5). Diese Spezifikation beschreibt, was
  Patrick sieht und was in Local Voice AI (Windows, macOS, iOS) gebaut wird.
- Baut auf: Seite „Chat“ (lva#339, U3 `conversations.db`), globaler Sprachassistent (lva#325), Buch-Gespräch
  (lva#307), Projekte (lva#365 P4), Bibliothek-Sync, Geräte-Kopplung per QR, Agentenbrücke, M365-Konto.

## 1. Ziel in einem Satz

Ein Assistent, mit dem Patrick auf PC, Mac und iPhone im selben Gespräch schreibt oder spricht, der sofort antwortet
wenn er kann, ansagt wenn er arbeitet, Zwischenstände zeigt und vorliest, sich unterwegs nachsteuern lässt, Mails,
Kalender und Projektakten kennt, Antwortentwürfe ablegt (nie sendet) und Claude Code Aufgaben übergibt.

## 2. Was es nicht wird

- Keine neue App, keine neue Seite. Die Seite „Chat“ (Desktop) und der Sprach-Chat (iPhone) werden ausgebaut.
- Kein Senden von Mails, kein Annehmen von Terminen, keine Kalenderänderung durch die KI.
- Keine zweite Skill- oder Konnektor-Verwaltung (Skills aus Corporate LLM, ADR-0021).
- Kein Rohaudio im Sync oder Log.

## 3. Nutzungsszenarien (Abnahme aus Patricks Sicht)

| # | Szenario | Erwartung |
|---|---|---|
| N1 | Am PC tippen „Fass mir das Projekt Wolff.ai zusammen“, dann aufs iPhone wechseln und per Sprache fragen „und was ist der nächste Schritt?“ | iPhone zeigt den Verlauf bis zur letzten Antwort, versteht den Bezug, antwortet gesprochen. Beide Beiträge erscheinen danach am PC. |
| N2 | „Hallo, hörst du mich?“ | Antwort in < 0,8 s, ohne Werkzeug. |
| N3 | „Wann habe ich nächste Woche Zeit?“ | Sofortige Ansage „Ich schaue in deinen Kalender.“, dann Antwort mit Zeiten aus dem Kalender. |
| N4 | „Ist eine neue Mail gekommen? Was will der Absender?“ | Ansage, Zusammenfassung der neuesten relevanten Mail mit Absender, Anliegen, Frist. |
| N5 | „Bereite mir eine Antwort vor.“ (Anfrage für eine Schulung an einem Termin) | Auftragskarte erscheint: Termin prüfen → Konditionen aus früheren Verträgen/Abrechnungen → Vorbereitungsaufwand → Empfehlung → Entwurf. Teilergebnisse werden gezeigt und gesprochen. Am Ende „Entwurf liegt in deinen Entwürfen“. Zahlen ohne Quelle sind `[bitte prüfen]`. |
| N6 | Während N5 läuft: „Achte darauf, dass ich im Dezember keine Präsenztermine mache.“ | „Nehme ich auf.“ Auftrag läuft weiter, berücksichtigt den Hinweis im nächsten Schritt; Karte zeigt „Hinweis übernommen“. |
| N7 | Während N5 läuft: „Wie spät ist es?“ | Sofortige Antwort, Auftrag läuft weiter. |
| N8 | „Wie ist der Stand im Projekt? Was kommt als Nächstes?“ | Offene PRs, CI-Zustand, zuletzt gemergt, nächste Pakete; gesprochen in ≤ 4 Sätzen, Details am Bildschirm. |
| N9 | „Gib Claude Code die Aufgabe, im Local-Voice-Repo den Fehler X zu beheben.“ | Rückfrage „Soll ich im Repo local-voice-ai eine Claude-Code-Sitzung starten?“ → „Ja“ → Auftrag läuft, Fortschritt sichtbar, PR-Link am Ende. |
| N10 | Frage, auf die es keinen Beleg gibt („Was habe ich 2019 für X berechnet?“, nichts vorhanden) | „Dazu finde ich nichts in deinen Unterlagen.“ Keine geschätzte Zahl. |

## 4. Oberfläche

Grundsatz: ChatGPT-Muster, Fluent 2 (`@wai/ui`), dichte Platznutzung, eine Fläche für Text und Sprache.

### 4.1 Desktop (Windows/macOS) – Seite „Chat“

- **Verlauf** wie heute; Beiträge zeigen Modus (gesprochen = kleines Wellensymbol), tatsächliches Modell, Quellen-Chips.
- **Eingabefeld** mit **Kontextleiste** (unten im Feld, eine Zeile): Modell ▾ · Projekt ▾ · ＋ (Werkzeuge,
  Konnektoren, Skills, Anhang) · rechts Diktat (Mikro) und Gespräch (Welle) bzw. Senden. Bestehende Modellwahl
  (`voice_speaker`) wandert in „Modell ▾“.
- **Sprachleiste** (bei laufender Sprache über dem Eingabefeld, nicht als eigene Seite): Pegel, Zustand
  (hört zu · denkt · arbeitet · spricht), Stumm, Beenden, Zeit bis zum ersten Ton im ⓘ.
- **Auftragskarte** im Verlauf: Titel, Schritte mit Zustand, Teilergebnisse mit Quelle, Hinweise („übernommen“),
  Budget, Knöpfe „Hinweis geben“, „Abbrechen“. Eingeklappt einzeilig mit Fortschritt.
- **Gerätehinweis** im Kopf des Gesprächs: „Läuft gerade am iPhone – hier übernehmen?“

### 4.2 iPhone

- Vorhandener Sprach-Chat wird zum Assistenten: Verlauf (Text + gesprochen), Eingabefeld mit Text und Sprachknopf,
  Kontextleiste als Chips über dem Feld (Modell, Projekt, ＋).
- Sprachmodus im Vollbild wie heute (Freisprechen), Auftragskarten dort kompakt als Fortschrittszeile.
- Gesprächsliste = synchronisierter Verlauf aller Geräte; Kennzeichnung „am PC begonnen“.
- Modell nicht auf dem Gerät → Beitrag zeigt „über PC beantwortet“ (Relay, wie Buch-Gespräch).

### 4.3 HTML-Vorschau

Vor jedem UI-PR eine Vorschau (390/768/1440/2560 px, hell/dunkel) unter
`apps/local-voice/docs/vorschau/assistent/` zur Codex-Abnahme. Erste Vorschau mit A1: Kontextleiste,
Sprachleiste, Auftragskarte, Gerätehinweis, iPhone-Verlauf.

## 5. Verhalten

### 5.1 Einstufung und Ansagen (A2)

Stufen S0–S3 nach ADR-0028 E4. Im Code: `@wai/voice` bekommt `einstufen(aeusserung, kontext) → {stufe, absicht,
konfidenz}`; LVA liefert Regeln (Bedienwörter, `agent/route`-Absichten) und das lokale Klassifikationsmodell.
Ansagen kommen aus einer Vorlagentabelle je Absicht und Sprache (DE/EN), z. B. `kalender` → „Ich schaue in deinen
Kalender.“ Bei Unsicherheit gilt die höhere Stufe. Der Sprecher bekommt in S0 einen Systemprompt ohne Werkzeuge und
mit Faktensperre; erkennt der Ausgabeprüfer Zahlen, Daten oder Namen aus Patricks Daten in einer S0-Antwort, wird die
Antwort verworfen und als S1 neu eingestuft.

### 5.2 Aufträge und Nachsteuern (A3)

Nach ADR-0028 E5. Desktop: Rust-Crate `wai-auftraege` (vendort wie `wai-voice`), Tauri-Ereignis `assistent-auftrag`.
iPhone: Aufträge laufen standardmäßig auf dem gekoppelten PC; das iPhone zeigt und steuert über Sync. Läuft kein PC,
laufen nur S0–S2 auf dem iPhone, S3 wird angeboten „Auftrag startet, sobald dein PC erreichbar ist“.

Gesprochene Zwischenstände: höchstens alle 20 s, nie während Patrick spricht, nie mitten in einer Antwort; Einstellung
„alles / Meilensteine / nur Ende“ (Standard Meilensteine).

### 5.3 Fortsetzen über Geräte (A1)

- Sync-Sammlungen `unterhaltung`, `beitrag`, `sitzung_steuerung` (ADR-0028 E3), Pull-Filter im Hub wie
  `booktalk_relay` (wai-portal-PR nötig).
- Migration: `conversations.db` bleibt Speicher am Desktop; neue Spalten für Sync-Kennung, Kette, HLC. iOS: `Entry`
  mit `conversationId` wird in einen Speicher im selben Schema überführt; alte Einträge bleiben lesbar.
- Konflikt: zwei Geräte antworten auf denselben Vorgänger → Verzweigung sichtbar („2 Fortsetzungen“), Patrick wählt.

### 5.4 Mail und Kalender (A5)

- M365-Konto: neue Fähigkeiten „Mails lesen“, „Kalender lesen“, „Entwürfe anlegen“ (je einzeln einschaltbar,
  Zustimmung erweitern wie heute). „Mail senden“ bleibt eine Fähigkeit der Besprechungs-Follow-ups und wird dem
  Assistenten nie als Werkzeug angeboten.
- Kontext für Konditionen: Projektordner (P4) mit Verträgen/Abrechnungen, Suche mit Quellenangabe.
- Entwurf: Antwort im Verlauf der Originalmail (Reply-Entwurf), Kategorie „KI-Entwurf“, im Protokoll.

### 5.5 Claude Code (A6)

- `projekt_stand`: GitHub lesend (eigene Repos, Token im Geheimnisspeicher). Zusammenfassung gesprochen kurz,
  Bildschirm ausführlich mit Links.
- `claude_code_auftrag` lokal: Start einer Sitzung im gewählten Repo-Ordner über die installierte CLI im
  Strom-JSON-Modus; Ja-Bindung an Repo + Aufgabe + 60 s Ablauf; Abbruch jederzeit; Ergebnis/PR-Link als
  `teilergebnis`.
- Cloud-Weg: Spike (ADR-0028 F4); bis dahin Issue mit Kennzeichnung `auftrag-sprache`.

## 6. Qualitätstore je Ausbaustufe

| Stufe | Tor |
|---|---|
| A1 | Rundlauf PC→iPhone→PC mit 50 Beiträgen ohne Verlust/Dopplung; Verzweigungstest; Migration alter Gespräche; kein Klartext im Hub |
| A2 | ≥ 200 Äußerungen DE (+ 100 EN): 0 Faktenfragen als S0, p95 S0 < 0,8 s (PC) / < 1,2 s (iPhone), Ansage ab S1 < 0,8 s |
| A3 | Nachsteuern in 20 Läufen ohne Neustart übernommen; Wiederanlauf nach App-Neustart; Drosselung der Ansagen |
| A4 | Projektkontext nur bei Freigabe, Widerruf wirkt sofort (P4c-Regeln) |
| A5 | 30 synthetische Anfragen (`example.*`): 0 Sendevorgänge, jede Zahl im Entwurf mit Quelle oder `[bitte prüfen]`, Kalenderkonflikte erkannt |
| A6 | Kein Start ohne Ja; Ja an Repo+Aufgabe gebunden; Abbruch beendet Prozessbaum |

## 7. Zuordnung zu bestehenden Arbeiten

| Bestehend | Verhältnis |
|---|---|
| lva#325 Globaler Sprachassistent | Kontextquelle „App-Inhalte“ für den Assistenten; P4 Latenzmessung geht in A2 auf |
| lva#307 Buch-Gespräch | bleibt eigener Bereich „Buch“; nutzt später Einstufung (A2) und Relay-Muster für A1 |
| lva#365 P4c Projekt-KI | Voraussetzung für A4 |
| ADR-0021 S5/S6 | Orchestrator ist Basis von A2/A3; S6 (Corporate LLM als Werkzeugquelle) liefert Skills für A4 |

## 8. Offene Punkte an Codex

F1–F5 aus ADR-0028 sowie: Reihenfolge A1 → A2 → A3 (vorgeschlagen) oder A2 zuerst, weil Tempo/Qualität Patricks
höchste Priorität hat?
