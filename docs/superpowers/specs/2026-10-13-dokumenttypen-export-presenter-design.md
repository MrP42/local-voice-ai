# Dokumenttypen, gestalteter Export und WAI-Presenter-Schnittstelle – Entwurf

Stand 13.10.2026, Etappe 0 (kein Produktcode). Zweig `feat/dokumenttypen-export`, Basis
`feat/vorlesen-uebersetzung` @7f68c7d4.

## Ziel

Aus einem Transkript entstehen neben dem Protokoll weitere Dokumenttypen (Management Summary,
Kurzfassung, …). Jedes Dokument lässt sich als Markdown/Text, Word, PDF und PowerPoint im
hinterlegten Briefkopf exportieren und als Paket an den WAI Presenter übergeben; der Presenter kann
umgekehrt eine Besprechung anfordern, und Local Voice AI erzeugt dafür fehlende Dokumente.

## Bestand (gesichtet, daran wird angeschlossen)

| Baustein | Fundstelle | Folgerung |
|---|---|---|
| Dokumente je Besprechung | `meeting_documents` (`kind`, `template_id`, `body`, `version` je `kind`, `generation_metadata_json`), `store.rs:2025 insert_document` | **Keine Migration nötig**: neue Typen sind neue `kind`-Werte |
| Protokoll-Motor | `minutes.rs` (Vorlage → JSON-Schema je Abschnitt → Einzeldurchlauf, wenn alles passt, sonst Blöcke mit Halbieren; Belege werden geprüft; `MinutesRunGuard`, Fortschritt, Stopp, `DocBasis` für Fassung/Sprache) | Dokumenttypen laufen **durch denselben Motor**; Cloud/Abo = ein Aufruf, lokal = Blöcke (`single_pass_budget_chars`) gilt automatisch |
| Varianten des Motors | `minutes/project.rs`: `ProjectKind::{Minutes, Summary}` ändert nur Prompt-Regeln (`SUMMARY_RULES`) und Eintragsgrenze (`entry_cap_for`) | Genau dieses Muster verallgemeinern: ein **Dokumentprofil** statt einer Parallelwelt |
| Vorlagen | `notes/templates.rs`: 10 eingebaute Besprechungsarten (`builtin:<key>`), `TemplateSpec {context, sections[Text/Tasks]}`, Grenzen 10 Abschnitte / 500 Zeichen je Anweisung | Dokumenttypen nutzen `TemplateSpec` für ihre Abschnitte; die Besprechungsart bleibt Kontext |
| Anzeige | `MinutesView.tsx` (Reiter `minutes`, filtert `kind === "minutes"`), Menü `MeetingActions.tsx` | Typ-Umschalter im selben Reiter, kein weiterer Reiter je Typ |
| Export | `export.rs`: `parse_blocks` → Markdown/Text/Word (handgeschriebenes OOXML über `zip`); `pdf.rs`: PDF über WebView2 **nur Windows** (macOS: `Unavailable`) | Word ausbauen; PDF braucht einen plattformgleichen Weg |
| Follow-up-Mail | `followup.rs`, `mail.rs`, `FollowupDialog.tsx` (.eml, mailto) | Typ „Follow-up-Mail“ verweist auf den vorhandenen Dialog |
| Ziele/Rechte | `managers/integrations/` (Arten, Fähigkeiten, Recht aus/fragen/erlaubt, `gate`, Audit, Freigaben, `folder::Sandbox`), Seite Integrationen | Presenter wird **eine Integrationsart** mit Ordner-Sandbox und Rechten |
| Agentenbrücke | `agent_bridge/` (Named Pipe, **nur Windows**), MCP nur lesend | Für die Gegenrichtung ungeeignet (Mac, OneDrive); Dateiaustausch statt Pipe |
| Presenter | `D:\wai-presenter` (nur gelesen): Bibliothek = Ordner, Projekt = Ordner mit `vortrag.json` (`wai.vortrag/1`), `auftrag.md` (Publikum, Ziel, Rahmen, Owner-Entscheidungen, Material); liest heute `meetings.db` nur lesend für den Live-Assistenten | Übergabe als eigenes Paketformat; der Presenter legt daraus selbst ein Projekt an |

## 1. Dokumenttypen (Top 10)

Länge sind Richtwerte im Prompt plus Eintragsgrenze je Abschnitt; die Wortzahl wird nach dem Lauf
gemessen und bei Überschreitung in den Metadaten vermerkt (nie still gekürzt). Für alle gilt die
Protokoll-Regel: Namen, Zahlen, Termine, Beschlüsse nur aus dem Transkript, jeder Eintrag mit Beleg.

| # | Schlüssel | Titel | Zielgruppe | Länge / Tiefe | Aufbau (Abschnitte) |
|---|---|---|---|---|---|
| 1 | `protokoll` | Protokoll (ausführlich) | Teilnehmende, Ablage, Nachweis | 2–6 S. / hoch | wie heute nach Besprechungsvorlage (Allgemein, Vertrieb, …) – **unverändert, `kind = minutes`** |
| 2 | `management_summary` | Management Summary | Entscheider, nicht anwesend | ≤ 1 S., ~300 Wörter / gering, Wirkung statt Verlauf | Anlass und Ziel (2 Sätze) · Kernergebnisse (3–5) · Entscheidungen · Risiken und Handlungsbedarf · Empfehlung / nächste Schritte (Aufgaben) |
| 3 | `kurzfassung` | Zusammenfassung (kompakt) | alle Beteiligten, schneller Überblick | ≤ ½ S., ~180 Wörter / gering | Worum ging es (1–2 Sätze) · Wichtigste Punkte (3–6) · Ergebnis · Nächste Schritte (≤ 3) · Hinweis „Details im Protokoll“ |
| 4 | `entscheidungen_aufgaben` | Entscheidungen und Aufgaben | Umsetzende, Projektsteuerung | nur Listen / mittel | Entscheidungen (1 Satz je) · Aufgaben (Was/Wer/Bis) · Offene Punkte |
| 5 | `followup_mail` | Follow-up-Mail | Teilnehmende, Kunde | ≤ 250 Wörter / gering | **vorhandener Follow-up-Dialog** (Betreff, Dank, 3 Punkte, Vereinbarungen, Gruß); kein neuer Generator |
| 6 | `statusbericht` | Statusbericht | Auftraggeber, Lenkungskreis | 1–2 S. / mittel | Stand insgesamt (als Einschätzung gekennzeichnet, keine erfundene Ampel) · Erreicht · Nächste Meilensteine und Termine · Risiken und Probleme · Entscheidungsbedarf · Aufgaben |
| 7 | `briefing_abwesende` | Briefing für Abwesende | Kolleginnen, die übernehmen | ≤ 1 S. / mittel | Kontext (Anlass, wer sprach – nur genannte Namen) · Was du wissen musst (5–8) · Was sich geändert hat · Was von dir erwartet wird (Aufgaben) · Offene Fragen |
| 8 | `faq` | Fragen und Antworten | Stakeholder, Wissensablage | nach Bedarf / mittel | Gestellte Fragen mit Antwort aus dem Gespräch · Offen gebliebene Fragen |
| 9 | `lernzusammenfassung` | Lernzusammenfassung | Teilnehmende einer Schulung / eines Vortrags | 1–2 S. / mittel | Thema und Lernziele (nur genannte) · Kernbegriffe mit Erklärung · Wichtigste Aussagen · Beispiele aus dem Vortrag · Merksätze · Genannte Quellen |
| 10 | `vortragsgrundlage` | Vortragsgrundlage (für den Presenter) | Vortragende, Presenter-Studio | 1–2 S. / mittel | Thema und Kernbotschaft · Zielgruppe (wie genannt) · Ziel · Rahmen: Dauer, Termin, Ort, Format (nur genannt) · Inhaltliche Bausteine / Gliederungsvorschlag · Fakten und Zahlen mit Beleg (→ `faktencheck.md`) · Offene Punkte / Owner-Entscheidungen · Organisatorisches (Aufgaben) |

Nr. 10 ist absichtlich auf `auftrag.md` des Presenters zugeschnitten (Publikum, Ziel, Rahmen,
Owner-Entscheidungen, Material), damit der Import dort ohne Umdeutung gelingt.

## 2. Datenmodell und Motor

- **Katalog im Code** `managers/meetings/doc_types.rs` (wie `builtin_templates`; Schlüssel nie umbenennen):
  ```rust
  pub enum DocEngine { Minutes, Document, Followup }
  pub struct DocType { key: &'static str, title: &'static str, audience: &'static str,
      length: &'static str, engine: DocEngine, spec: TemplateSpec,
      rules: &'static str /* Prompt-Zusatz wie SUMMARY_RULES */, entry_cap: Option<usize>,
      max_words: Option<u32> }
  pub fn catalog() -> Vec<DocType>;  pub fn by_key(key: &str) -> Option<DocType>;
  pub fn doc_kind(key: &str) -> String; // "minutes" | "doc:<key>"
  ```
- **Speicherung:** `meeting_documents.kind = "doc:<key>"`, `template_id = "doctype:<key>"`,
  `body_format = "markdown@1"`, Metadaten wie `MinutesMeta` plus `doc_type`, `words`, `over_length`.
  Versionen zählen je Typ (vorhandene Logik je `kind`). Keine Schemaänderung, kein `user_version`.
- **Motor:** `minutes.rs` bekommt ein `DocProfile { kind, heading, rules, entry_cap, spec }`
  (Vorgabe = heutiges Protokoll, damit bestehende Aufrufer und Tests unverändert bleiben). Neu:
  `generate_document(app, store, meeting_id, key, &DocBasis, on_progress)`; die Besprechungsart
  (`resolve_template`) geht nur als Kontextsatz ein. Sperre bleibt **eine je Besprechung**
  (`MinutesRunGuard`): Protokoll und Dokument laufen nacheinander; zweiter Start → `minutes_busy`.
- **Abrechnung:** `Purpose::Document` (eigene Zeile in der Nutzung), Provenienz-Operation `document`.
- **Ereignis:** `MinutesEvent` erhält `doc_kind` in allen Varianten; `MinutesView` reagiert nur auf
  seinen `kind`.
- **Tauri-Befehle:** `meetings_document_types() -> Vec<DocTypeInfo>`,
  `meetings_generate_document(meeting_id, doc_type, doc_basis) -> MeetingDocument`; Zustand und
  Stopp über die vorhandenen `meetings_minutes_state` / `meetings_minutes_cancel`.
- **Oberfläche:** Reiter `minutes` heißt „Dokumente“ (Id bleibt, keine Migration gespeicherter
  Reiter). Darin eine Typleiste: vorhandene Dokumente als Chips (Protokoll zuerst) und „Neues
  Dokument ▾“ mit allen zehn Typen samt Zielgruppe/Länge als Unterzeile; Follow-up öffnet den
  vorhandenen Dialog. Im Menü der Besprechung „Dokument erzeugen …“ neben „Protokoll neu erzeugen“.

## 3. Export-Pipeline

Eine Quelle: der **gespeicherte Markdown-Text** (Handkorrekturen über `update_document_body` gelten
mit). `export::parse_blocks` wird um `Table` und `Todo` erweitert und liefert ein Blockmodell, aus
dem jedes Format rendert. Neuer Befehl `meetings_export_document_as(document_id, path,
letterhead_id?)`; das Format folgt der Endung (`md`, `txt`, `docx`, `pdf`, `pptx`), Dateiname wie
bisher (`<Titel> – <Dokumenttyp>.docx`, Sonderzeichen ersetzt). Geschrieben wird nach `.part` und
umbenannt (Muster `pdf.rs`).

| Format | Weg | Begründung | Lizenz |
|---|---|---|---|
| Markdown / Text | vorhanden | – | – |
| Word | vorhandener OOXML-Schreiber, ergänzt um `header1.xml`/`footer1.xml`, Logo unter `word/media`, Seitenzahl-Feld, Tabellen, echte Aufzählung (`numbering.xml`), Farben/Schrift aus dem Briefkopf. Optional: **eigene Word-Vorlage** (.docx/.dotx) – Kopf, Fuß, Formatvorlagen bleiben, nur der Körper wird ersetzt | keine neue Abhängigkeit, Tests vorhanden; Vorlage = „Briefkopf wie im Firmen-Word“ | – |
| PDF | **`krilla` 0.8** (reiner Rust, MIT OR Apache-2.0, PDF-Schreiber von Typst) + eigenes kleines Satzmodul: Zeilenumbruch, Seitenumbruch mit Kopf/Fuß je Seite, Tabellen, Logo (PNG/JPEG), Schrift eingebettet, PDF/A-2b | gleiche Ausgabe auf Windows und macOS, ohne Fenster/Browser testbar (Text per vorhandenem `pdf-extract` prüfbar); WebView2-PDF bleibt für den Besprechungs-Export unter Windows | MIT/Apache |
| PowerPoint | handgeschriebenes OOXML über `zip` (eine Masterfolie, drei Layouts) | wie Word: keine Abhängigkeit; Inhalte deterministisch aus dem Dokument | – |

Verworfen: **Typst als Bibliothek** (Apache-2.0, beste Typografie, aber großer Abhängigkeitsbaum mit
Plugin-Laufzeit, Bibliografie, Syntaxhervorhebung – Größe nicht gemessen, daher nicht gewählt);
**Word/PowerPoint-Automation** (nur Windows); **WebView2 + WKWebView** (zwei Wege, macOS hier nicht
testbar); **docx-rs** (vorhandener Schreiber reicht).

PDF „ohne inhaltliche Änderbarkeit“: PDF ist festes Layout; eine Schreibsperre per Passwort ist
keine echte Sicherheit und wird nicht angeboten (siehe Owner-Entscheidungen).

**PowerPoint-Aufbau:** Titelfolie (Dokumenttyp, Besprechungstitel, Datum, Logo) · je Abschnitt eine
Folie mit höchstens 6 Punkten, Rest auf Folgefolien „(2/2)“ · Aufgaben als Tabelle Was/Wer/Bis ·
Entscheidungen als eigene Folie · Sprechernotizen = voller Abschnittstext. Kein zusätzlicher
Modellaufruf, nichts wird hinzuerfunden.

## 4. Briefkopf (austauschbar)

- **Profil** `lva.briefkopf/1` als Ordner `<App-Daten>/briefkoepfe/<id>/` mit `briefkopf.json`,
  optional `logo.png|jpg` (≤ 2 MB, ≤ 4000 px) und `vorlage.docx`:
  `{schema, id, name, firma:{name, zeilen[≤4]}, logo:{datei, breite_mm, position:links|rechts},
  farben:{akzent, akzent_text, text, gedaempft, linie}, schrift:{titel, text}, fusszeile:{zeilen[≤3],
  seitenzahl}, word_vorlage?}`. Farben als `#RRGGBB` geprüft.
- **Mitgeliefert:** „Neutral“ (ohne Logo, Grau) und „Wolff Applied AI“ (Akzent `#FFDD00` nur als
  Fläche mit Ink `#111418`, W schwarz auf hell; Werte gespiegelt aus `design-system_wolffappliedai`),
  optional „Ingenieurbüro Wolff“ (Navy `#004B85`).
- **Schriften:** Word/PowerPoint **Segoe UI** (Medienregel des Design-Systems); PDF bettet
  mitgelieferte OFL-Schriften ein (Archivo Überschriften, IBM Plex Sans Fließtext mit Kursiv) –
  Eintrag in `THIRD_PARTY_NOTICES`.
- **Einstellung:** `document_letterhead_id` (Vorgabe `neutral`) im Reiter **Ausgabe**, neue Gruppe
  „Dokumente“: Briefkopf wählen, Felder bearbeiten, Logo/Word-Vorlage wählen, Profil
  importieren/exportieren (`.lvbriefkopf.zip`), Vorschau-PDF. Kein neuer Reiter.
- Briefkopf-Code ruft nie `settings::get_settings`; die Wahl kommt als Argument (Muster `usage.rs`).

## 5. Übergabe an den WAI Presenter

**Grundsatz:** Local Voice AI schreibt nie in das Projektformat des Presenters, sondern ein eigenes,
versioniertes Paket in einen **Austauschordner**; der Presenter zeigt es unter „Eingang“ und legt
daraus selbst ein `wai.vortrag/1`-Projekt an. Jede Seite besitzt ihr Format.

**Integrationsart `Presenter`** (Seite Integrationen, wie Ordner-Ziel): Wurzel = Austauschordner
(Sandbox `folder::Sandbox`), Umfang des Katalogs, Fähigkeiten `presenter.send` (schreibend, Nutzer
ohne Freigabe) und `documents.generate` (Anfragen des Presenters, Aufrufer `agent_local`, Vorgabe
**fragen**). Rechte, Freigabedialog, Audit: vorhandenes Tor.

```
<Austausch>/
  katalog.json                       LVA: freigegebene Besprechungen, nur Metadaten
  pakete/<datum>-<slug>-<id8>[-vN]/  LVA: ein Paket je Sendung
    besprechung.json                 Manifest, zuletzt geschrieben
    transkript.md  transkript.json   Segmente {start_ms, end_ms, sprecher, text}
    dokumente/<typ>.md [.docx .pdf .pptx]
  anfragen/<anfrage-id>.json         Presenter: Anforderung
  antworten/<anfrage-id>.json        LVA: Stand, Fortschritt, Ergebnis
```

- **Manifest `wai.besprechung/1`:** `{schema, paket_id (ULID), besprechung_id, erstellt, quelle:
  {app, version}, titel, datum, dauer_ms, sprache, teilnehmende[] (nur bestätigte Namen, sonst
  Kanalbezeichnung), vorlage, dokumente:[{typ, titel, datei, formate[], version, erzeugt, modell,
  belegt}], transkript:{dateien[], segmente, fassung}, dateien:{<pfad>: sha256}, datenklasse:
  "vertraulich"}`. Kein Audio, keine Geheimnisse, keine Notizen ohne Wahl.
- **Schreiben:** in `pakete/.tmp-<paket_id>/`, Prüfsummen, Manifest zuletzt, dann umbenennen.
  Erneutes Senden: neuer Ordner `-v2`; ältere LVA-Pakete derselben Besprechung räumt LVA nach
  Erfolg ab (nur Ordner mit eigenem gültigem Manifest). Der Presenter nimmt ein Paket erst, wenn alle
  Prüfsummen stimmen (OneDrive synchronisiert Dateien einzeln).
- **Senden aus LVA:** im Dokumente-Reiter und im Besprechungsmenü „An WAI Presenter senden …“
  (nur bei eingerichteter Integration): Dokumente und Formate wählen, fehlende Dokumente vorher
  erzeugen (mit Fortschritt und Stopp). Befehl `presenter_send(meeting_id, doc_types[], formats[])
  -> {paket}`.
- **Gegenrichtung:** Der Presenter liest `katalog.json` (`wai.besprechung-katalog/1`: `id, titel,
  datum, dauer_ms, transkript: bool, dokumente[]`) und schreibt
  `{schema:"wai.besprechung-anfrage/1", id, erstellt, von, besprechung_id, dokumente[], formate[],
  sprache?, neu_erzeugen:false}`. LVA prüft alle 5 s (nur wenn die App läuft und die Integration
  an ist): Größe ≤ 16 KiB, Schema, Id-Muster `[0-9A-Z]{26}`, Besprechung im Katalog-Umfang,
  höchstens 5 bekannte Typen, Formate aus {md, docx, pdf, pptx}, Alter ≤ 7 Tage, jede Id nur
  einmal. Dann Tor (`documents.generate`, Freigabe im Dialog mit Titel, Typen, Modell und
  Cloud-Hinweis), fehlende Dokumente erzeugen (vorhandene wiederverwenden, außer `neu_erzeugen`),
  Paket schreiben. Antwort `wai.besprechung-antwort/1`: `stand ∈ {wartet_auf_freigabe, laeuft,
  fertig, abgelehnt, abgebrochen, fehler}`, `fortschritt {schritt, fertig, gesamt}` (höchstens alle
  2 s), `paket`, `code`. Löscht der Presenter die Anfrage, bricht LVA ab.
- **Presenter-Seite** baut Eingang, „Als Vortrag anlegen“ (Vortragsgrundlage → `auftrag.md`,
  Fakten → `faktencheck.md`, Paket nach `quelle/besprechung/`) und die Anforderung. Auftrag dafür:
  `docs/presenter-uebergabe-auftrag.md` (Etappe 4); im Presenter-Repo wird hier nichts geändert.

## 6. Fehlerfälle

| Fall | Verhalten |
|---|---|
| Lauf abgebrochen / App beendet | nichts geschrieben (Motor schreibt am Ende in einer Transaktion); Paket bleibt `.tmp-…`, wird beim nächsten Start abgeräumt |
| Zweiter Start, Protokoll läuft | `minutes_busy`, Anfrage bleibt `laeuft`/wartet |
| Modell gesperrt (Compliance), kein Anbieter, RAM knapp | vorhandene Codes (`no_provider`, `memory_low`, …), Antwort `fehler` mit Code |
| Platte voll / Ordner weg / OneDrive offline | Schreibfehler → kein halbes Paket, Code `presenter_write_failed`, Integration zeigt Status |
| Pfadflucht, Junction, Gerätename in Ids | Sandbox lehnt ab (vorhandene Tests der Ordner-Sandbox) |
| Fremde/manipulierte Anfrage | Schema- und Umfangsprüfung, Freigabe „fragen“, Audit; Inhalte der Anfrage sind nie Prompt-Text |
| Logo defekt / zu groß, Word-Vorlage ohne `document.xml` | Profil wird beim Speichern abgelehnt; Export fällt nie still auf „Neutral“ zurück |
| Überlänge | Dokument bleibt, Hinweis im Kopf und in den Metadaten |

## 7. Testplan

- **Rust:** Katalog (Schlüsselmuster, `validate_spec`, eindeutig, 10 Einträge); Prompt enthält
  Regeln und Grenzen; `generate_document` mit Attrappen-LLM (`kind`, Version, Stopp schreibt nichts,
  Sperre geteilt, Einzeldurchlauf/Blöcke); Blockmodell (Tabelle, Aufgaben); Word (Teile, Kopf/Fuß,
  Logo-Beziehung, Vorlagen-Einsetzen, XML-Maskierung); PDF (`%PDF-`, Seitenzahl, Text per
  `pdf-extract`, Schrift eingebettet, Umbruch langer Tabellen); PowerPoint (Folienzahl je Abschnitt,
  Teile gültig); Briefkopf-Prüfung; Paket (atomar, Prüfsummen, `-v2`, Aufräumen); Anfragen
  (Übergröße, Schema, unbekannter Typ, Ablauf, doppelte Id, Abbruch durch Löschen, Tor „fragen“).
- **Playwright** (`LV_DEV_PORT=1660`, Attrappen): Typleiste mit 10 Typen, Erzeugen mit
  Fortschritt/Stopp, Export-Menü mit 5 Formaten, Briefkopf-Gruppe unter Ausgabe, Sendedialog.
- **Sichtprüfung:** PDF/PPTX in Bilder (PyMuPDF/`pdftoppm`), Word über `python-docx` gelesen.

## 8. Etappen und Pakete

| Paket | Umfang | Akzeptanz (Befehl → Ergebnis) | hängt ab |
|---|---|---|---|
| E1a | `doc_types.rs`, `DocProfile` im Motor, `Purpose::Document` | `cargo test --lib doc_types minutes` → grün | – |
| E1b | Befehle, `bindings.ts`, Typleiste, i18n de/en | `pnpm tsc --noEmit` + `playwright test tests/documents.spec.ts` → grün, Screenshot | E1a |
| E1c | Beleg: Management Summary einer echten Besprechung aus einer **Kopie** der Datenbank | Markdown-Datei im Scratchpad | E1a |
| E2a | Briefkopf-Profil, Speicher, Einstellungsgruppe | `cargo test --lib letterhead` → grün | – |
| E2b | Blockmodell + Word mit Briefkopf / eigener Vorlage | `cargo test --lib export` → grün, .docx je Typ | E2a |
| E2c | PDF mit `krilla`, Schriften, Notices | `cargo test --lib pdf_doc` → grün, PDF → PNG angesehen | E2a |
| E3 | PowerPoint | `cargo test --lib pptx` → grün, .pptx + PNG-Vorschau | E2b |
| E4a | Integrationsart Presenter, Paket, Senden, UI | `cargo test --lib presenter` → grün, Paketordner | E1, E2 |
| E4b | Katalog, Anfragen, Antworten, Freigabe; Codex-Review des Vertrags | Tests grün, Review-Befunde eingearbeitet | E4a |
| E4c | `docs/presenter-uebergabe-auftrag.md` | Datei liegt vor | E4b |

Aufwand (Schätzung): E1 ~200 kTok, E2 ~250, E3 ~100, E4 ~200 – zusammen ~750 kTok.

## 9. Risiken

- `minutes.rs` (4 131 Zeilen) wird auch von anderen Zweigen geändert: Eingriff klein halten
  (Profil als Parameter mit Vorgabe), früh mergen.
- PDF-Satz ist Eigenbau: Silbentrennung fehlt (Flattersatz links), sehr lange Tabellenzellen.
- Austauschordner in einem geteilten OneDrive gibt vertrauliche Inhalte weiter: Warnhinweis beim
  Einrichten, Datenklasse im Manifest.
- Neue `kind`-Werte erscheinen nicht automatisch in Suche, Chat und Besprechungs-Export: in E2
  den Besprechungs-Export ergänzen, Suche/Chat bewusst später.
- `krilla` verlangt Rust ≥ 1.92 (vorhanden: 1.97.1); Binärgrößenzuwachs in E2c messen.

## 10. Owner-Entscheidungen (als begründete Annahme gesetzt)

1. **Reitername „Dokumente“** statt „Protokoll“ (Id `minutes` bleibt). Annahme: ja.
2. **Follow-up-Mail** bleibt der vorhandene Dialog, kein zweiter Generator. Annahme: ja.
3. **Mitgelieferte Briefköpfe:** „Neutral“ + „Wolff Applied AI“ (+ „Ingenieurbüro Wolff“). Wer die
   App an Kunden gibt, sieht die WAI-Vorlage – Alternative: nur „Neutral“ mitliefern und das
   WAI-Profil als Datei importieren. Annahme: alle drei mitliefern, Vorgabe „Neutral“.
4. **Rechtsangaben im Fuß** (USt-IdNr., Bank) nicht in Besprechungsdokumenten. Annahme: nein.
5. **PDF-Schriften:** OFL-Schriften (Archivo, IBM Plex Sans) einbetten. Annahme: ja.
6. **Keine PDF-Passwortsperre**, dafür PDF/A-2b. Annahme: so.
7. **Austauschordner:** vom Nutzer gewählt, Vorschlag `Dokumente/WAI Presenter/Austausch`
   (lokal); OneDrive nur bewusst (Mac-Presenter). Annahme: so.
8. **Katalog-Umfang:** alle abgeschlossenen Besprechungen, nur Titel/Datum/Dauer/Typen; Inhalte erst
   nach Freigabe. Alternative: nur gewählte Projekte. Annahme: alle, umschaltbar.
9. **Anfragen des Presenters:** Recht „fragen“ (jede Anfrage einzeln freigeben). Annahme: ja.
10. **Kein Audio** im Paket. Annahme: ja.
11. **Format-Namensraum** `wai.besprechung/1` (Familie von `wai.vortrag/1`). Annahme: ja.
