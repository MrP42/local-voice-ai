# Besprechungsplanung – Konzept und Entscheidungen

Stand 12.11.2026 · Epic [lva#534](https://github.com/MrP42/local-voice-ai/issues/534) · Klassifizierung: intern

Organisationsneutral: Das Modul nennt keine Organisationen, Produkte Dritter oder Personen.
Alles Organisationsspezifische (Erscheinungsbild, erlaubte Sprachmodelle, Aufbewahrung,
Regelwerke) kommt aus Einstellungen: Briefkopf, Richtlinienprofil, Kalenderquellen.

## 1. Ziel und Ablauf

Ein durchgehender Ablauf in einer App. Outlook bleibt führend für Zeit, Ort und
Teilnehmende, die App für die Planungsinhalte.

| # | Schritt | Wer führt | Ergebnis |
|---|---|---|---|
| 1 | Planen | App | Ziel, erwartetes Ergebnis, Agenda mit Zeitboxen, Rollen, Unterlagen, Hinweise, Klassifizierung, „Transkription geplant ja/nein“ |
| 2 | Termin anlegen | Outlook über Adapter | Termin mit Einladungstext im gewählten Briefkopf, Teilnehmende, Ort/Beitritts-Link; Verknüpfung App ↔ Termin |
| 3 | Synchron halten | Outlook führt Zeit, Ort, Teilnehmende | Verschiebung oder Absage in Outlook aktualisiert die Planung |
| 4 | Durchführen | App | Hinweisfenster vor Beginn; „Besprechung starten“ mit Einwilligungs-Check; Agenda-Zeitboxen sichtbar |
| 5 | Transkribieren | App, immer lokal | Transkript mit Sprechern und Notizen |
| 6 | Protokoll | Sprachmodell aus der Allowlist des Richtlinienprofils (lokal, Server der Organisation oder freigegebene API) | Protokoll entlang der Agenda, Ziel-Abgleich, Aufgaben mit Verantwortlichen und Terminen, Belege |
| 7 | Versenden | Outlook | Entwurf an alle Teilnehmenden des Termins; Versand durch den Menschen, nie automatisch |

## 2. Befund im Repo (main 08ef511)

Schritte 4 bis 7 existieren weitgehend; das Modul ist vor allem Schritt 1 bis 3 plus Verdrahtung.

| Baustein | Vorhanden | Wo | Lücke |
|---|---|---|---|
| Kalender lesen (ICS, Graph mit PKCE, Sync) | ja | `managers/calendar/{ics,graph,service}.rs` | Termin-Schlüssel `quelle:uid:start_ms` ändert sich bei Verschiebung; Graph-`id` wird nicht gespeichert |
| Besprechung ↔ Termin | ja, Schnappschuss | `meeting_calendar_links` | Verknüpfung erst beim Aufnehmen; kein Nachziehen bei Verschiebung |
| Hinweisfenster vor Beginn | ja | `meeting_prompt.rs`, `calendar/reminder.rs` | kennt keine Planung, keinen Einwilligungs-Check je Termin |
| Leerer Eintrag ohne Audio | ja | `meetings/empty.rs` | Träger der geplanten Besprechung |
| Einwilligung (§ 201 StGB) | ja, global | `consent_confirmed_at` | je Termin und Person fehlt |
| Vorlagen, KI-Notizen, Protokoll mit Belegen | ja | `meetings/notes/templates`, `minutes.rs` | Agenda als terminbezogene Vorlage fehlt |
| M365-Konto mit Freigabe-Tor und Audit | ja | `managers/integrations/m365/` | `event_create.rs` nur ganztägige Fristen; kein Mail-Entwurf |
| Follow-up-Mail, `.eml`, mailto | ja | `meetings/followup.rs`, `mail.rs` | Empfänger nicht aus Termin-Teilnehmenden |
| Automationen | ja | `managers/workflows/` | Motor für Schritt 4 bis 7 |
| Aufbewahrung Audio „bis Protokoll steht“ | ja | `retention.rs` | im strengen Richtlinienprofil Pflicht |
| Briefkopf-Profile | ja | `managers/letterhead.rs` | Quelle des Einladungs-Erscheinungsbilds |
| Planungsformular | nein | bestehender HTML-Besprechungsgenerator (außerhalb des Repos) | als React-Komponente portieren |

Lizenz: `vendor/wai-*` ist proprietär. Die Kernlogik liegt deshalb in einem eigenen Crate
ohne `wai-*`-Abhängigkeit, damit das Modul auch in einer offenen Ausgabe bleibt.

## 3. Architektur

### 3.1 Bausteine

| Baustein | Art | Inhalt |
|---|---|---|
| `crates/besprechungsplanung` | neu (Rust, eigenständig) | Datenmodell, Migration, Speicher, Regelprüfung, Einladungstext mit Marke, Abgleich, Einwilligungs-Tor; 26 Tests, `cargo test` in Sekunden |
| `src-tauri/src/managers/planning/` | neu | Adapter-Schnittstelle `OutlookAdapter` mit drei Umsetzungen (3.3), Sync-Tick, Audit |
| `src-tauri/src/commands/meeting_planning.rs` | neu | Tauri-Befehle: anlegen, ändern, prüfen, Vorschau, an Outlook übergeben, verknüpfen, abgleichen, starten, Protokoll übergeben |
| `src/components/planung/` | neu (React) | Port des Generators: Formular, Vorschau, Regelhinweise, Status verknüpft/verschoben/abgesagt/Konflikt |
| `calendar/model.rs`, `graph.rs` | geändert | Graph-`id` (ImmutableId) und `changeKey` mitführen |
| `meeting_prompt.rs`, `reminder.rs` | geändert | Planung im Hinweisfenster; Start mit/ohne Transkription |
| `minutes.rs`, `notes/templates` | geändert | terminbezogene Vorlage aus der Agenda; Kopf Ziel/Ergebnis/erreicht? |
| `integrations/m365/` | erweitert | `event_upsert.rs` (Besprechung mit Teilnehmenden), `mail_draft.rs` (Entwurf, Scope `Mail.ReadWrite`) |
| Richtlinienprofil „streng“ | neu, in `managers/compliance` | LLM-Allowlist, Pflicht-Aufbewahrung, Redeanteile aus, keine externen Anbieter ohne Freigabe |

### 3.2 Datenmodell

Drei Tabellen in der Besprechungs-Datenbank, nur `CREATE`: `meeting_plans`,
`meeting_plan_links`, `meeting_plan_consents` (Schema: `crates/besprechungsplanung/src/store.rs`).
Teilnehmende bleiben in `calendar_event_attendees` und `meeting_participants` (Outlook führend).
Einwilligungen sind je Planung und Person (normalisierte E-Mail-Adresse) gespeichert.

### 3.3 Outlook-Anbindung: drei austauschbare Adapter

Entscheidung: Schnittstelle sofort für alle drei, Umsetzung in der Reihenfolge **C → B → A**.
EWS scheidet aus (Abschaltung für Exchange Online).

| | A: Microsoft Graph | B: Outlook lokal (COM) | C: Datei + ICS-Feed |
|---|---|---|---|
| Voraussetzung | Exchange Online, Entra-App, Admin-Zustimmung im Mandanten der Organisation | klassisches Outlook unter Windows | nur Outlook |
| Termin anlegen | `POST /me/events` mit Teilnehmenden und HTML-Text; Marke zusätzlich als `singleValueExtendedProperty` | `AppointmentItem` anlegen, Text einsetzen, `Display()`; Mensch ergänzt Beitritts-Link und sendet | App legt HTML in die Zwischenablage, Marke `lva-plan:<id>` im Fuß; Mensch legt den Termin an |
| Verschiebung erkennen | `calendarView/delta` alle 5 Min. plus gezielter Abruf vor Beginn; `Prefer: IdType="ImmutableId"` | Abruf über `GlobalAppointmentID` je Sync-Tick | ICS-Feed (vorhanden), Zuordnung über die Marke; Verzögerung beim Anbieter bis zu Stunden |
| Protokoll übergeben | `POST /me/messages` als Entwurf, `webLink` öffnen | `MailItem` mit `HTMLBody`, `Display()` | `.eml` mit `X-Unsent: 1` (vorhanden) |
| Bewertung | sauberste Lösung, Governance-Hürde | pragmatischer Start, keine Mandanten-Zustimmung | Notlösung, immer verfügbar, Sync träge |

### 3.4 Marke und Sync-Regeln

Die App schreibt einen eigenen Block: Kopfzeile „— Besprechungsplanung —“ bis zur Zeile
`lva-plan:<id>`. Sichtbare Textmarke statt HTML-Kommentar, weil Outlook Kommentare beim
Bearbeiten entfernt und der ICS-Feed nur Text liefert. Inhalte werden einzeilig gemacht und
können keine fremde Marke einschleusen. Die Prüfsumme läuft nur über Buchstaben und Ziffern,
damit Outlooks HTML→Text-Wandlung keinen falschen Konflikt erzeugt.

| Feld | Führend | Verhalten bei Änderung |
|---|---|---|
| Beginn, Ende, Ort, Beitritts-Link | Outlook | App übernimmt, Status „verschoben“ bis zur Bestätigung; Erinnerung und Automationen rechnen neu |
| Teilnehmende, Antwortstatus | Outlook | übernehmen; Neue bekommen Einwilligung „offen“ |
| Absage | Outlook | Status „abgesagt“, Erinnerung aus, Planung bleibt; Wiederherstellung wird erkannt |
| Ziel, Ergebnis, Agenda, Rollen, Hinweise | App | App schreibt nur ihren Block mit `If-Match`/`changeKey`; wurde der Block in Outlook geändert: Konflikt, nie überschreiben |
| Termin gelöscht oder Marke weg | – | Status „verwaist“, Neu-Verknüpfung per Auswahl |
| Planung schon durchgeführt | – | Kalender ändert den Status nicht mehr |

Bei Adapter C schreibt die App den Block nach dem Einfügen nie wieder; dort zählt nur die Marke.
Aus dem Bestand: nie den ganzen Termintext ersetzen, keine Dubletten (`transactionId`, Marke),
unklarer Ausgang wird gemeldet statt wiederholt, jede schreibende Aktion über das Freigabe-Tor mit Audit.

### 3.5 Durchführung

- Hinweisfenster 5 Min. vorher (einstellbar) mit Ziel, Agenda, Rollen und Stand der Einwilligungen.
- „Besprechung starten“ öffnet Agenda-Zeitboxen (Restzeit je Punkt) und Notizblock. Ist Transkription
  geplant, verlangt der Knopf vorher die Bestätigung je anwesender Person. Fehlt eine Zustimmung oder
  liegt eine Ablehnung vor, startet die Besprechung nur mit Notizen (`consent::start_modus`).
- Beginnt die Besprechung ohne App, bietet die vorhandene Mikrofon-Erkennung die verknüpfte Planung an.

### 3.6 Protokoll und Versand

- Terminbezogene Vorlage: je Agendapunkt ein Abschnitt, dazu Ergebnisse, Entscheidungen, Aufgaben
  (wer, was, bis wann); Kopf mit Ziel, erwartetem Ergebnis und „erreicht ja / teilweise / nein“.
- Sprachmodell nur aus der Allowlist des Richtlinienprofils; externe Anbieter mit dem vorhandenen Hinweis vorab.
- Der Versandtext enthält nur das freigegebene Protokoll, keine Transkriptauszüge.
- Entwurf an alle Teilnehmenden, Betreff „Protokoll: <Titel> (<Datum>)“, Klassifizierungszeile wie in der Einladung.
- Nach Freigabe: Audio löschen, Transkript nach eingestellter Frist.

## 4. Datenschutz und Regelwerk (organisationsneutral)

Voraussetzung für jeden Einsatz mit Transkription ist die Freigabe der App als KI-Werkzeug durch die
zuständige Stelle der Organisation; die Planung ohne Transkription braucht sie nicht.

| Thema | Umsetzung im Modul | Grundlage |
|---|---|---|
| Keine Aufzeichnung von Inhaltsdaten über den Zweck hinaus | Audio nur als Puffer bis das Protokoll steht; kein Audio-Export | interne Regelung zur Telekommunikation; DSGVO Art. 5 |
| KI-Mitschrift nur mit freigegebenem Werkzeug | Freigabe durch die zuständige Stelle; bis dahin nur Pilot mit Zustimmung | interne KI-Richtlinie; EU AI Act |
| Einwilligung aller | je Termin und Person dokumentiert; Hinweis bereits in der Einladung | § 201 StGB; DSGVO Art. 6, 7 |
| Keine Leistungs- und Verhaltenskontrolle | Redeanteile im strengen Profil aus; keine Auswertung über Personen hinweg | Personalvertretungsrecht (in BW § 75 Abs. 4 Nr. 11 LPVG) |
| Datenminimierung, Klassifizierung | Klassifizierung je Planung, im Termin und im Protokoll; Links statt Anhänge | Klassifizierungsrichtlinie der Organisation; DSGVO Art. 5 |
| Geheimnisse | Token nur im Betriebssystem-Tresor, nie in DB oder Log | Kryptokonzept der Organisation |
| Protokollierung | Audit jeder schreibenden Outlook-Aktion (Bestand) | Protokollierungsregeln der Organisation |
| DSFA | vor einem Pilot mit Transkription: Verarbeitungsverzeichnis und DSFA (Sprachdaten) | DSGVO Art. 30, 35 |
| Server- oder Cloud-Sprachmodell | nur Server der Organisation oder freigegebene API mit AV-Vertrag | Cloud-Richtlinie der Organisation |

Rechtsbezüge abgeleitet, nicht juristisch geprüft.

## 5. Umsetzungsplan

Arbeitsweise nach `AGENTS.md`: eigener Zweig je Paket, PR, keine Formatierläufe über fremde Dateien.

| Paket | Inhalt | Abnahme |
|---|---|---|
| P0+P1 | Konzept im Repo; Crate mit Modell, Migration, Speicher, Regeln, Einladung, Abgleich, Einwilligung | `cargo test` im Crate grün |
| P2 | Planungsoberfläche (Port Generator), Regelhinweise, Vorschau | Vorschau entspricht dem Generator; HTML-Vorschau zur Abnahme |
| P3 | Adapter C + App-Verdrahtung (Migration, Befehle, Bindings) | Termin aus dem Feed wird verknüpft; Verschiebung aktualisiert die Planung |
| P4 | Hinweisfenster, „Besprechung starten“, Einwilligungs-Check, Agenda-Zeitboxen | Start mit/ohne Transkription, Einwilligung dokumentiert |
| P5 | Terminbezogene Protokollvorlage, Ziel-Abgleich, Entwurf an Teilnehmende | Protokoll entlang Agenda, Entwurf mit allen Teilnehmenden |
| P6 | Adapter B: Outlook COM (Windows) | Termin per `Display()`, Verschiebung über `GlobalAppointmentID`, Mail-Entwurf |
| P7 | Adapter A: Graph | Tests gegen den vorhandenen Test-Server |
| P8 | Doku, Playwright-Abnahme, Einwilligungs- und Datenschutztexte | Abnahme durch den Owner |

## 6. Entscheidungen

| Datum | Entscheidung |
|---|---|
| 12.11.2026 | Modul im bestehenden Repo, kein Fork. |
| 12.11.2026 | Adapter-Reihenfolge C → B → A. |
| 12.11.2026 | Planung auch ohne Transkription nutzbar (reiner Besprechungsplaner, keine KI-Freigabe nötig). |
| 12.11.2026 | Kernlogik als eigenes Crate ohne `wai-*`; organisationsspezifisches nur über Briefkopf und Richtlinienprofil. |

## 7. Offene Fragen

1. Exchange online oder on-premises, klassisches oder neues Outlook am Zielarbeitsplatz (entscheidet über Adapter B).
2. Welcher Sprachmodell-Endpunkt ist für einen Pilot freigegeben?
3. Wann werden Personalvertretung und Datenschutz eingebunden?
4. Lizenz einer offenen Ausgabe (EUPL-1.2 oder MIT).
