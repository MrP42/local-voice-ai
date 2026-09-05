# Stimmen-Baukasten — Entwurf

Stand: 05.09.2026. Entschieden im Brainstorming mit Patrick, noch nicht umgesetzt.
Projekt: `apps/local-voice`. Grundlage: `main` @ `caf1b22` (v0.15.0).

## 1. Warum

Neue Stimmen entstehen heute nur auf zwei Wegen: eine Referenz aufnehmen
(`record_reference_start/stop`) oder eine WAV importieren (`tts_import_voice`).
Wer keine Aufnahme hat, kommt an keine neue Stimme — und wer eine Figur wie
Pyrion sprechen lassen will, hat gar keine Vorlage.

Ziel ist ein Assistent, der aus **Name und Beschreibung** mehrere Stimm-Kandidaten
erzeugt, die man **anhört, vergleicht, nachregelt** und dann als Stimme
**speichert** — mit einem Zwischenstand, der einen Absturz übersteht. Dazu eine
**Bibliothek** fertiger Startpunkte (Erzähler, Opa/Oma, Drache, Eule, Magier …),
und als erste konkrete Stimme **Pyrion**.

## 2. Der harte technische Rahmen

Diese Punkte sind am Code und an der Fish-Audio-Doku geprüft, nicht angenommen.
Sie bestimmen den ganzen Entwurf:

- **Fish-Speech kennt keine Konditionierung auf eine Beschreibung.** Eine Stimme
  entsteht so (`mod.rs::ensure_seed_reference`): ohne Referenz erzeugt der Server
  aus einem **Seed** eine zufällige Stimme; diese WAV wird als Referenz abgelegt
  (`sample.wav` + `sample.lab`) und ab dann geklont. Der Seed ist der einzige
  Regler für die Stimm*identität*.
- **Die Beschreibung wirkt indirekt, aber messbar**: sie bestimmt den
  Referenz-Probesatz und dessen Emotions-Tags. Beim Zero-Shot-Klonen überträgt
  sich die Prosodie der Referenz — ein langsam und schwer gesprochener
  Referenzsatz ergibt eine andere Stimme als ein munterer. Zusätzlich füllt sie
  die Metadaten (Default-Tags, Stil, Farbe, Beschreibung), die bei jedem
  späteren Vorlesen gelten.
- **Keine Tonhöhen-/Formantverschiebung vorhanden.** `dsp.rs` kann nur Biquads,
  `enhance.rs` nur EQ. Ein „tiefer" wird deshalb über **Resampling der Referenz**
  erreicht (siehe 4.3).
- **Der lokale fish-speech-Server kennt vier Routengruppen** (geprüft an
  `C:\AI\fish-speech\tools\server\views.py`): `/v1/health`, `/v1/vqgan/*`,
  `/v1/tts`, `/v1/references/*`. **Kein** `voice-design`, **keine** mitgelieferte
  Stimmenbibliothek.
- **Fish-Audio-Cloud bleibt draußen** (Entscheidung Patrick, 05.09.2026). Weder
  Voice Design (`POST /v1/voice-design`, Cloud) noch die Discovery-Stimmen werden
  angebunden. Die Hörproben der Discovery-Modelle werden **nicht** heruntergeladen
  und lokal nachgeklont: die Rechte, die Fish Audio sich beim Stimmeigner sichert
  (`licensed=true`), gelten für ihre Plattform und reichen nicht an uns weiter.
- **Kinderstimmen entstehen nicht aus Korpus-Aufnahmen.** Eine kindlich klingende
  Stimme darf nur aus der Lotterie kommen, nie aus einem Datensatz mit echten
  Kindern.

## 3. Die Quellen einer Stimme

Alle Quellen münden in **dieselbe** Registry (`meta.json` je Stimmenordner,
`registry.rs`) und sind danach nicht mehr zu unterscheiden — Sprecher-Chips,
Dialoge und Vorlesen funktionieren für alle gleich.

| Quelle | Etappe | `VoiceOrigin` |
|---|---|---|
| Seed-Lotterie aus einer Beschreibung | 1 | `Seed(i64)` |
| Eigene Aufnahme oder WAV-Import | 2 | `Recording` |
| Rezept aus der Bibliothek (füllt die Lotterie vor) | 3 | `Seed(i64)` |
| Freier Sprecher (CC0/gemeinfrei) per Download | 3 | `Recording` |

Die Herkunft bleibt damit unverändert; `VoiceOrigin` braucht keine neue Variante.

## 4. Etappe 1 — der Assistent

### 4.1 Ablauf aus Sicht des Nutzers

1. In der Stimmenkarte unter **Vorlesen** auf „Neue Stimme erschaffen".
2. Name und Beschreibung eingeben (Pyrion-Beschreibung als Beispiel hinterlegt).
3. Der Assistent schlägt **Probesatz und Tags** vor — aus der Beschreibung über
   den vorhandenen LLM-Pfad, mit einem brauchbaren Rückfall ohne LLM (siehe 4.4).
4. „Kandidaten erzeugen" liefert N Stimmen (Standard 6), jede aus einem anderen
   Seed. Fortschritt je Kandidat, Abbruch möglich.
5. Anhören, verwerfen, nachwürfeln; Probesatz, Tags und **Tiefe-Regler** ändern
   und erneut erzeugen.
6. Den Treffer wählen, Anzeigename und Farbe bestätigen, **speichern**.

### 4.2 Absturzsicherheit

Der Entwurf lebt **im Backend**, nicht im React-Zustand:

```
<fish_dir>/builder/<draft_id>/
  draft.json          # Zustand, atomar geschrieben (temp + rename)
  cand_<seed>.wav     # je Kandidat eine Datei
```

`draft.json` wird nach **jeder** Zustandsänderung geschrieben: Kandidat fertig,
Regler bewegt, Text geändert (entprellt), Auswahl getroffen. Beim Start listet
die Stimmenkarte offene Entwürfe und bietet „fortsetzen" an, samt der bereits
erzeugten Kandidaten. Ein abgebrochener Lauf hinterlässt keinen halben Zustand:
geschrieben wird erst, wenn eine Kandidaten-WAV vollständig auf der Platte liegt.

Aufräumen: Entwürfe älter als 30 Tage werden beim Start entfernt; „verwerfen"
löscht sofort. Der Ordner liegt bewusst neben den Stimmen und nicht im TEMP,
damit ein Neustart des Rechners ihn nicht mitnimmt.

### 4.3 Der Tiefe-Regler

Ein Faktor `depth` von 1,00 bis 1,15 streckt die Kandidaten-WAV per **linearer
Resampling-Interpolation** (neu in `dsp.rs`, mit Tests). Strecken um 15 % senkt
Tonhöhe und Formanten gemeinsam — die klassische „tiefer und älter"-Methode.

Angewandt wird der Faktor auf die **Referenz vor dem Klonen**, nicht auf die
Ausgabe: das Modell synthetisiert danach neu und rechnet die Artefakte des
Resamplings weitgehend weg. Der Faktor wird im Entwurf gespeichert und ist
jederzeit ohne Neuerzeugung umstellbar, weil die Original-WAV erhalten bleibt.

Grenze, die im Text stehen muss: über etwa 15 % klingt es künstlich. Das
entscheidet das Ohr, nicht die Software.

### 4.4 Beschreibung → Probesatz und Tags

Der LLM-Pfad (`tagging.rs`, Ollama lokal oder Anthropic) bekommt die Beschreibung
und liefert einen Probesatz (max. 150 Zeichen, in der Rolle gesprochen) plus
passende Tags aus der vorhandenen Registry (`src/lib/tags/`).

**Ohne LLM muss der Assistent trotzdem funktionieren** — kein Anbieter
konfiguriert, kein Ollama gestartet, keine Internetverbindung. Rückfall: ein
neutraler deutscher Probesatz und keine Tags, beides frei editierbar. Der
LLM-Schritt ist Komfort, keine Voraussetzung.

### 4.5 Backend-Schnitt

Neues Modul `managers/tts/builder.rs`:

```rust
pub struct BuilderDraft {
    pub id: String,              // ULID
    pub display_name: String,
    pub description: String,
    pub probe_text: String,
    pub tags: Vec<String>,
    pub depth: f32,              // 1.00 .. 1.15
    pub candidates: Vec<Candidate>,
    pub selected: Option<i64>,   // Seed des gewählten Kandidaten
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct Candidate { pub seed: i64, pub file: String, pub created_at: i64 }
```

Funktionen: `create_draft`, `load_draft`, `list_drafts`, `save_draft`,
`delete_draft`, `generate_candidates` (mit Fortschritts-Event und Abbruch über
denselben watch-Channel-Weg wie das Auto-Tagging), `preview_candidate`
(Kandidat mit aktuellem `depth` als WAV liefern), `commit_draft` (gewählten
Kandidaten als Stimme speichern).

`commit_draft` benutzt die vorhandene Speicherstrecke: der Kandidat wird
`sample.wav`, der Probesatz `sample.lab`, dazu `write_seed_marker` und
`registry::write_meta`. Der bestehende `save_seed_voice_v2` wird dafür so
zerlegt, dass der WAV-Ursprung ein Parameter ist — kein zweiter Pfad, der
auseinanderlaufen kann.

Neue Commands in `commands/tts.rs`, Bindings von Hand nachziehen
(tauri-specta regeneriert nur beim Debug-Lauf).

### 4.6 Frontend-Schnitt

`components/settings/tts/builder/`:

- `VoiceBuilder.tsx` — der Assistent als Panel in der Stimmenkarte, drei
  Abschnitte (Beschreiben → Kandidaten → Speichern) untereinander, kein Wizard
  mit Zurück-Knopf: der Zustand liegt ohnehin im Backend.
- `CandidateCard.tsx` — Kandidat mit Abspielknopf, Seed, Auswahlring.
- `useBuilderDraft.ts` — lädt/speichert den Entwurf, hört auf das
  Fortschritts-Event.

Kein neuer Menüpunkt und kein neuer Reiter (Regel aus `AGENTS.md`): der
Assistent gehört in die Stimmenkarte, wo Stimmen ohnehin verwaltet werden.

### 4.7 Pyrion

Die Beschreibung von Patrick wird als erstes Bibliotheks-Rezept hinterlegt
(`src/lib/voices/recipes.ts`, in Etappe 1 nur dieser eine Eintrag) — tiefe,
alte, würdevolle Männerstimme, langsames Tempo, Tags Richtung `[slow]`,
`[serious]`. Der Assistent startet damit vorbefüllt.

**Die Abnahme kann nur Patrick machen**: ob ein Kandidat nach Pyrion klingt,
hört ein Mensch. Etappe 1 endet mit „erzeuge Kandidaten und hör sie an", nicht
mit einer grünen Testbilanz. Reicht die Lotterie nicht, sind der Tiefe-Regler
und danach Etappe 2 (eigene Aufnahme) die Hebel.

## 5. Etappe 2 — Referenz-Import als zweite Quelle

Eigene Aufnahme oder WAV-Datei als Kandidat in denselben Entwurf: Aufnahme über
den vorhandenen Weg, Import über einen Dateidialog. Dazu ein **Zuschnitt** auf
die brauchbaren 10–30 Sekunden (Start/Ende ziehen, Vorschau), weil eine zu lange
oder verrauschte Referenz die Klonqualität senkt. Der Tiefe-Regler gilt auch
hier — damit wird „selbst einsprechen und absenken" der zuverlässige Weg zu
Pyrion, falls die Lotterie nicht trifft.

## 6. Etappe 3 — Bibliothek

Zwei Sorten Einträge, in derselben Kachelwand:

1. **Rezepte** (~20): Name, Beschreibung, Probesatz, Tags, Metadaten — füllen den
   Assistenten vor und starten die Lotterie. Der einzige Weg für Fabelwesen
   (Drache, Eule, Magier, Pyrion), weil es dafür kein Korpusmaterial gibt.
   Optional ein festgeschriebener Seed, sobald ein Treffer gefunden ist: das
   Rezept merkt sich, was einmal gut klang.
2. **Freie Sprecher** (~10): echte Referenzaufnahmen unter CC0 oder gemeinfrei —
   [Thorsten-Voice](https://www.thorsten-voice.de/en/) (CC0, deutsch,
   [SLR95](https://www.openslr.org/95/) neutral und
   [SLR110](https://www.openslr.org/110/) emotional), LibriVox (gemeinfrei) für
   Erzählerstimmen. Ein-Klick-Download über den vorhandenen Katalog-Weg mit
   SHA-Pins (`models.rs`, `gen_catalog.py`), danach lokal geklont.

Jeder Eintrag nennt Herkunft und Lizenz in der Oberfläche. Was keine klare
Lizenz hat, kommt nicht in die Bibliothek.

## 7. Etappe 4 — dauerhafte Klangregler je Stimme

`VoiceMeta` bekommt ein optionales Feld `sound` (Tempo, Lautheitskorrektur,
EQ-Wärme über `enhance`). Es gilt bei **jedem** Vorlesen dieser Stimme, nicht nur
im Assistenten. Anknüpfpunkte in der Pipeline sind vorhanden: `playback_gain`
schlüsselt bereits je Stimme, der Player skaliert das Tempo live.

Eigene Etappe, weil es als einziger Teil in die Vorlese-Pipeline eingreift und
deshalb eigene Tests und eine eigene Hörprobe braucht.

## 7b. Etappe 5 — Stimmen verwalten: umbenennen, bearbeiten, exportieren, importieren

Nachgereicht von Patrick am 05.09.2026. Betrifft alle Stimmen, egal aus welcher
Quelle.

**Bearbeiten** heisst: Anzeigename, Farbe, Beschreibung, Sprache, Default-Tags
und Stile aendern. Das ist reines Schreiben in die `meta.json`; Commands und
Validierung existieren bereits (`tts_get_voice_meta`, `tts_set_voice_meta`,
`registry::validate_meta`). Es fehlt nur die Oberflaeche — das ist das seit
v0.14.0 offene Paket S2 (Stimmen-UI v2).

**Umbenennen** hat zwei Ebenen, die auseinandergehalten werden muessen:

- Der **Anzeigename** steht in der `meta.json` und ist frei aenderbar. Aber:
  Sprecher-Marker in gespeicherten Texten (`<Anna>`, `Anna:`) loesen ueber
  genau diesen Namen auf. Wer umbenennt, entwertet die Marker in seinen Texten
  — die Oberflaeche muss davor warnen und den alten Namen als Alias anbieten,
  statt es stillschweigend geschehen zu lassen.
- Die **voice_id** ist der Ordnername und steckt zusaetzlich in der Einstellung
  `tts_voice`, in `seed.txt`-Nachbarschaft und im WAV-Cache-Schluessel. Ein
  Umbenennen der id ist deshalb ein Umzug: Ordner verschieben, Einstellung
  nachziehen, Cache-Eintraege dieser Stimme verwerfen. Angeboten wird es nur
  ausdruecklich, nicht als Nebenwirkung des Anzeigenamens.

**Exportieren**: eine Stimme als ein einzelnes `.lvvoice`-Archiv (ZIP mit
`meta.json`, `sample.wav`, `sample.lab`, Avatar und `seed.txt`, falls
vorhanden). Das `zip`-Crate ist vorhanden und wird im Meeting-Export bereits
so benutzt (`managers/meetings/export.rs`) — dasselbe Muster, kein neues
Werkzeug. Dateidialog ueber `tauri-plugin-dialog`, das schon eingebunden ist.

**Importieren**: dasselbe Archiv zurueck. Beim Import wird geprueft, ob der
Anzeigename schon vergeben ist (`registry::other_voice_names` +
`validate_meta`), und bei Kollision ein neuer Name vorgeschlagen statt die
vorhandene Stimme zu ueberschreiben. Ein Archiv mit Pfaden ausserhalb des
Zielordners wird abgelehnt (Zip-Slip); der Meeting-Export-Code hat dafuer
bereits die Haltung, die uebernommen wird.

Damit wird eine Stimme teilbar: erschaffen auf einem Rechner, benutzt auf
einem anderen — und ein Backup der eigenen Stimmen ist eine Datei je Stimme.

**Bewusst offen gelassen:** Beim Umbenennen der voice_id werden die
WAV-Cache-Eintraege der alten id NICHT verworfen. Der Schluessel enthaelt die
id, die Eintraege werden nach dem Umzug also nie wieder getroffen und laufen
ueber die normale Cache-Pflege heraus — der Effekt ist verlorener Plattenplatz
auf Zeit, kein falscher Klang. Ein gezieltes Verwerfen waere ein Eingriff in
`compile_cache.rs` und lohnt den Aufwand erst, wenn jemand haeufig umbenennt.

## 8. Was NICHT gebaut wird

- Keine Anbindung der Fish-Audio-Cloud (Voice Design, Discovery-TTS).
- Kein Herunterladen und Nachklonen fremder Discovery-Stimmen.
- Keine Kinderstimmen aus Korpusmaterial.
- Keine echte Tonhöhen-/Formantverschiebung (Phasenvocoder) — der
  Resampling-Weg reicht für „tiefer und älter" und kostet keine neue Bibliothek.
- Kein Wizard mit Zurück-Navigation; der Zustand liegt im Backend, die drei
  Abschnitte stehen untereinander.

## 9. Prüfung

- **Rust**: Einheitstests für Resampling (Länge, Tonhöhenverhältnis,
  Randfälle), Entwurfs-Persistenz (atomar, Wiederaufnahme nach simuliertem
  Absturz, Aufräumen alter Entwürfe), `commit_draft` (Zielordner vollständig
  oder gar nicht — die Regel aus `save_seed_voice_v2`).
- **TypeScript**: Der Assistent muss ohne LLM und ohne laufenden Server
  bedienbar bleiben (Rückfalltexte, klare Fehlermeldung statt leerer Liste).
- **Manuell, nur Patrick**: klingt ein Kandidat nach Pyrion; wo liegt die
  hörbare Grenze des Tiefe-Reglers.

## 10. Aufwand

| Etappe | Schätzung |
|---|---|
| 1 Assistent + Pyrion | 130–170 kTok |
| 2 Referenz-Import | 60–80 kTok |
| 3 Bibliothek | 80–110 kTok |
| 4 Klangregler | 80–100 kTok |
| 5 Verwalten (umbenennen, bearbeiten, exportieren, importieren) | 90–120 kTok |

Meldung bei 50 % und 80 % des jeweiligen Etappenrahmens.
