# Terminfindung – Konzept und Entscheidungen

Stand 25.11.2026 · Issue [lva#845](https://github.com/MrP42/local-voice-ai/issues/845) · Teil der Besprechungsplanung (Epic [lva#534](https://github.com/MrP42/local-voice-ai/issues/534)) · Klassifizierung: intern

Organisationsneutral wie das gesamte Modul: Erscheinungsbild nur über Briefkopf/Gestaltung,
keine Organisations-, Produkt- oder Personennamen im Code und in Standardtexten.

## 1. Ziel

Einen Termin für eine Besprechung finden, ohne manuelle Brüche:

1. In der Planung Zeitvorschläge wählen – mit sichtbarer Verfügbarkeit aller internen
   Teilnehmenden (Outlook-Kalender über Microsoft Graph, nur frei/belegt).
2. Externe (und auf Wunsch interne) Teilnehmende per HTML-E-Mail im Corporate Design fragen.
3. Antworten kommen automatisch zurück in die App (Postfach über Graph), ohne Abtippen.
4. Ist die Runde vollständig oder die Frist erreicht, bekommt die einladende Person eine
   Zusammenfassung per E-Mail mit Knopf „In Local Voice planen“ (Deep-Link) und eine
   Benachrichtigung in der App.
5. Die App empfiehlt den besten Zeitpunkt mit nachvollziehbarer Begründung; ein Klick legt den
   Termin an (bestehender Planungsweg: Outlook-Termin, Webex/Teams-Übergabe, Einladungstext).
6. Alle Befragten erhalten automatisch „Termin steht“ bzw. die Einladung; vorläufige
   Kalender-Blocker der übrigen Vorschläge werden entfernt.

Schritt 2 (später): Abstimmungsseite im Web wie Doodle, mit sichtbarem Abstimmungsstand.

## 2. Ablauf Schritt 1

| # | Schritt | Wer | Daten |
|---|---|---|---|
| 1 | Umfrage anlegen (aus einer Planung oder frei) | App | Titel, Dauer, Teilnehmende (intern/extern, Pflicht/optional), Frist |
| 2 | Verfügbarkeit laden | Graph `POST /me/calendar/getSchedule` | nur `free/tentative/busy/oof/workingElsewhere` je Intervall, keine Betreffzeilen |
| 3 | Vorschläge wählen | App (Raster mit Verfügbarkeit, Vorschlag „beste freie Zeiten“) | 2–8 Zeitfenster |
| 4 | Optional: vorläufige Blocker im eigenen Kalender (Standard aus) | Graph, `showAs=tentative` | Betreff „Vorbehalten: …“ |
| 5 | Anfrage-Mails senden | Graph `sendMail`, eine Mail je Person | HTML + Textteil, persönlicher Antwort-Code |
| 6 | Antworten einlesen | Graph Delta-Abfrage im Posteingang, nur Mails mit Code im Betreff | Zusagen je Vorschlag |
| 7 | Auswerten | Crate (deterministisch), bei Freitext lokales Sprachmodell mit Bestätigung | Rangliste mit Begründung |
| 8 | Zusammenfassung + Deep-Link | Graph `sendMail` an sich selbst, App-Benachrichtigung | nur Umfrage-ID im Link |
| 9 | Termin planen | bestehender Planungsweg (Termin anlegen, Konferenz-Übergabe) | — |
| 10 | Abschluss | Mails „Termin steht“, Blocker löschen, Umfrage schließen | — |

## 3. Antwortweg per E-Mail (Entscheidungsvorschlag)

Eine E-Mail kann ohne Webseite kein Formular zuverlässig übertragen (Formulare und Skripte
werden von Outlook und den meisten Clients entfernt). Deshalb arbeitet die Mail mit
vorbereiteten Antworten über `mailto:`-Knöpfe, die jeder Client versteht:

- **„Alle Termine passen“** – ein Knopf.
- **Je Vorschlag „Passt“** – öffnet eine Antwort mit nur diesem Vorschlag.
- **„Auswahl beantworten“** – öffnet eine Antwort mit vorausgefüllter Liste
  (`[x] 1  Di 02.12. 10:00–11:00`), in der man Kreuze setzt oder löscht.
- **„Keiner passt“** mit Freitextfeld für Gegenvorschläge.
- Normales „Antworten“ in Outlook funktioniert ebenfalls: Freitext wird erkannt
  (deterministischer Parser für Muster wie „1 und 3“, „nur Dienstag“, „ja/nein“; sonst lokales
  Sprachmodell, Ergebnis immer als **Vorschlag zur Bestätigung**, nie stillschweigend).

Der Betreff jeder Antwort trägt `[Terminfindung T-XXXX-YYYY]`. `T-XXXX` ist die Umfrage,
`YYYY` ein persönlicher Code (HMAC über Umfrage-ID und Empfänger, Schlüssel nur lokal).

**Robustheit:** Code fehlt (z. B. neue Mail statt Antwort) → Zuordnung über
`conversationId` und Absenderadresse. Absender passt nicht zum Code (weitergeleitet) →
Eintrag „zur Prüfung“, keine automatische Wertung. Mehrfachantwort → die jüngste gilt,
Verlauf bleibt sichtbar. Abwesenheitsnotizen werden erkannt und ignoriert.

**Nicht gewählt:** Outlook Actionable Messages (nur intern, Registrierung bei Microsoft nötig,
für Externe unbrauchbar). Kann später als zusätzlicher Weg für Interne folgen.

## 4. Interne Verfügbarkeit

- Graph `getSchedule` für die eigene Adresse und alle internen Teilnehmenden
  (Scope `Calendars.ReadBasic`; liefert nur frei/belegt nach den Freigaben der Kollegen).
- Darstellung im Planungsraster als Heatmap: je Zeitfenster „alle frei“, „n von m frei“,
  Belegungsart je Person bei Mausberührung. Keine Termintitel, keine Speicherung über die
  Laufzeit der Umfrage hinaus (Cache 15 min, nur im Speicher).
- Interne Teilnehmende müssen daher meist **gar nicht** gefragt werden; Schalter
  „Interne trotzdem fragen“ für Fälle, in denen der Kalender nicht gepflegt ist.
- Ohne Graph (ICS/COM-Adapter): eigener Kalender aus dem bestehenden Sync, Kollegen unbekannt
  → Raster zeigt „Verfügbarkeit unbekannt“.

## 5. Bewertung „bester Zeitpunkt“

Rein deterministisch im Crate, jede Zahl in der UI erklärbar:

| Kriterium | Gewicht |
|---|---|
| Pflicht-Teilnehmende nicht verfügbar | Ausschluss (bzw. ans Ende, falls keiner passt) |
| Zusage „passt“ | +3 je Person (Pflicht ×2) |
| „ginge notfalls“ / vorläufig belegt | +1 |
| Keine Antwort | 0, als Unsicherheit angezeigt |
| Interne frei laut Kalender | wie Zusage |
| Innerhalb Kernzeit der einladenden Person | +1 |
| Gleichstand | früherer Termin |

Ausgabe: Rangliste mit Begründungstext („alle 6 Pflicht-Teilnehmenden können, 1 optionale
Person fehlt“).

## 6. Deep-Link

- Schema `localvoice://terminfindung/<umfrage-id>` über `tauri-plugin-deep-link`
  (MIT/Apache-2.0), unter Windows im Installer registriert, Weitergabe an die laufende Instanz
  über das vorhandene `tauri-plugin-single-instance`.
- Der Link trägt **nur** die Umfrage-ID (Format geprüft), keine Codes, keine Personendaten.
  Er öffnet die Umfrage; jede Aktion (Termin anlegen, Mails senden) braucht einen Klick in der App.
- Fallback, falls Outlook das Schema blockiert: Textzeile „Code T-XXXX in Local Voice
  eingeben“ und die App-Benachrichtigung.

## 7. Sicherheit und Datenschutz

- Externe erhalten nur: Titel (abschaltbar → neutraler Betreff), Dauer, Vorschläge,
  Absender. Keine Teilnehmerliste, keine Agenda (Agenda erst mit der Einladung).
- Antworten werden nur aus Mails mit gültigem Code gelesen; der übrige Posteingang wird nicht
  ausgewertet. Mailinhalte bleiben lokal; Freitext geht nur an das lokale bzw. per
  Richtlinienprofil erlaubte Sprachmodell.
- Neue Scopes nur über neue Fähigkeiten im M365-Konto (`Verfügbarkeit` → `Calendars.ReadBasic`,
  `Antworten einlesen` → `Mail.Read`); jede Fähigkeit einzeln abschaltbar, Audit-Eintrag je Versand.
- Aufbewahrung: Umfrage und Antworten 30 Tage nach Abschluss löschen (Einstellung).
- Keine Inhalte ins Log, nur IDs und Zähler.

## 8. Gestaltung

- E-Mails nutzen **dieselbe Vorlagenbasis** wie Einladung und Erinnerung der Besprechungsplanung
  (Briefkopf/Gestaltung, tabellenbasiertes HTML, Textteil, Dunkelmodus-fest). Neue Vorlagen:
  `terminfindung_anfrage`, `terminfindung_erinnerung`, `terminfindung_zusammenfassung`,
  `terminfindung_termin_steht`.
- App: Reiter „Terminfindung“ in der Planung mit drei Zuständen – Vorschläge wählen
  (Raster + Heatmap), Antworten sammeln (Matrix Person × Vorschlag, live), Entscheiden
  (Empfehlung mit Begründung, „Termin planen“).

## 9. Schritt 2: Abstimmungsseite (Vorschlag)

Kleine eigenständige Komponente „Abstimmung“ mit **Ende-zu-Ende-Verschlüsselung**:

- Die App erzeugt je Umfrage einen Schlüssel; Titel, Vorschläge und Namen werden im Browser
  bzw. in der App verschlüsselt, der Schlüssel steht nur im Fragment des Links (`#k=…`), das nie
  an den Server geht. Der Server speichert nur Chiffretext, Zeitstempel und Stimmen-IDs.
- Teilnehmende geben einen frei gewählten Anzeigenamen an, keine E-Mail-Adresse.
- Abstimmungsstand für alle sichtbar (entschlüsselt im Browser).
- Löschung automatisch 14 Tage nach Abschluss.
- **Hosting:** Pilot als Modul im wai-portal (IONOS, vorhanden, PHP), weil die Daten dort nur
  verschlüsselt liegen. Für den Einsatz in einer Organisation dieselbe Komponente als
  Container für den eigenen Server (gleiche API). Die App spricht beide über eine
  konfigurierbare Adresse an und holt den Stand per Abruf (kein Webhook nötig).

## 10. Pakete

| Paket | Inhalt | Umsetzung |
|---|---|---|
| TF1 | Crate: Modell Umfrage/Vorschlag/Antwort, Antwort-Code, Parser, Bewertung, Mail-Vorlagen (Text) | Codex |
| TF2 | Graph: Fähigkeiten Verfügbarkeit + Antworten, `getSchedule`, Delta-Abruf, Migration, Befehle | Codex |
| TF3 | Deep-Link: Plugin, Installer-Registrierung, Single-Instance-Weitergabe | Codex |
| TF4 | HTML-Vorlagen der vier Mails im Corporate Design (gemeinsame Basis) | Claude Opus |
| TF5 | UI Terminfindung (Raster, Antwortmatrix, Empfehlung, Benachrichtigung) | Claude Opus |
| TF6 | Abschluss: Termin anlegen, „Termin steht“, Blocker entfernen, Aufbewahrung | Codex |
| TF7 | Schritt 2 Abstimmungsseite | später |

## 11. Entscheidungen (Codex, 25.11.2026)

1. Antwortweg: `mailto`-Knöpfe + Freitext; keine Actionable Messages.
2. Posteingang-Abruf alle 2 min, nur solange eine Umfrage offen ist.
3. Vorläufige Blocker standardmäßig **aus** (abschaltbar an).
4. Schritt 2: Pilot im wai-portal mit Ende-zu-Ende-Verschlüsselung, Schlüssel im Link-Fragment.
5. Der persönliche Code hat 4 Zeichen (≈ 20,7 Bit) und ist kein Geheimnis: Antworten zählen nur bei passender
   Absenderadresse, Versuchslimit je Absender, Kollisionsprüfung beim Anlegen (TF2).
6. Reihenfolge: TF4 gemeinsam mit der Design-Engine lva#841 (Profil-ID, Platzhaltervertrag); TF1–TF3
   parallel; TF5 nach UI-Freigabe; TF6 nach der Terminübergabe.

### Begründungen aus TF1 (Codex)

1. Schritt 1 nutzt `mailto` und Freitext; Actionable Messages bleiben später optional.
   Vier Base36-Zeichen haben nur 36^4 = 1.679.616 Möglichkeiten (20,7 Bit) und sind
   als alleiniger Zugangsschutz zu kurz: Absenderbindung, Versuchslimit und Prüfung
   bei Weiterleitung sind in TF2 Pflicht. ID-Kollisionen beim Anlegen abfangen,
   Codes nie ohne Umfrage-ID und Adresse zuordnen (bei 100 Codes ca. 0,29 %
   Kollisionswahrscheinlichkeit); für einen späteren Web-Zugang längere Tokens verwenden.
2. Postfach alle zwei Minuten abrufen, ausschließlich solange eine Umfrage offen ist;
   Delta-Cursor nutzen und bei Fehlern bzw. Drosselung den Abstand erhöhen (TF2).
3. Vorläufige Blocker standardmäßig aus; bei Bedarf ausdrücklich aktivieren.
   So belegt eine Umfrage nicht ungefragt mehrere Zeitfenster im Kalender (TF6).
4. Schritt 2 als wai-portal-Pilot mit E2E-Verschlüsselung, Schlüssel nur im
   Link-Fragment; anschließend dieselbe API als selbst hostbarer Container (TF7).
