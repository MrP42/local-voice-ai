# P4c KI-Wege im Projekt (Plan, lva#365)

Stand 10.11.2026, Basis `origin/main` 32b706eb (nach #443). Grundlagen:
`apps/local-voice/docs/plans/2026-11-08_projekte-p4.md` (Task P4c, Entscheidungen),
Spec `docs/superpowers/specs/2026-11-07-buch-ordner-und-projekte.md` §5.2, §5.4, §9,
PR #382 (P4a), #383 (P4b) und #443 (D3-Vorbau, P4c-Anschlussstelle).

Nur Plan. Es wurde nichts gebaut und nichts gemessen. Zahlen in diesem Dokument sind Startwerte, keine Messwerte.

## Ziel

Alle KI-Wege im Projekt beziehen ihren Kontext ausschließlich über ein zentrales **Versandtor**
(`projekte::tor`). Das Tor ist die einzige Stelle, die `ProjektDienst::kontext_fuer_ki` aufruft.
Vor jedem Modellaufruf erledigt es Folgendes:

- Es prüft `stand` frisch.
- Es bestimmt das Ziel aus der Konfiguration.
- Es bereinigt den gesamten Prompt.
- Es reserviert das Budget in Token.

Nach dem Aufruf verwirft es verspätete Antworten. Erst wenn alle Wege angeschlossen und getestet sind, hebt das letzte
Paket die Sperre `PROJEKT_KI_FREIGEGEBEN` auf.

## 1 Inventur der KI-Wege

Heute nutzt **kein** Weg den Projektkontext: `kontext_fuer_ki` (`projekte/dienst.rs`) ist gesperrt, einziger
Aufrufer ist der Test `projekt_ki_pfade_liefern_ohne_p4c_freigabe_nichts` (`projekte/tests_p4.rs`). Die Tabelle
zeigt, wie die Wege heute Kontext, Verlauf und Ziel beziehen. Daraus folgt, wo das Tor ansetzen muss.

| # | Weg | Einstieg (Datei:Funktion) | Kontext heute | Verlauf/Zustand | Ziel (Modell) heute | P4c |
|---|---|---|---|---|---|---|
| W1 | Sprachgespräch, Stufe 1, und Chat mit Texteingabe (`components/voice-call/Composer.tsx`) | `voice_call/mod.rs:voice_call_chat` | App-Kontext `SpeakerContext` (globaler Index, `app_context/service.rs:bind_for_speaker_quellen`, nur an lokalen Empfänger). Notizbuch über `commands/voice_tools.rs:schutz_pruefen`. Einbettung in die letzte Nutzernachricht (`with_context_message`). | Verlauf kommt bei jedem Turn **per IPC** (`messages`). Gespeichert wird er in `conversations.db` (`conversations/store.rs`, U3); die Beiträge schreibt das Frontend über `conversation_append_entry`. Die letzte Ausgabe merkt sich `assistent::tor::ausgabe_merken`. | `settings.for_function(FN_VOICE_SPEAKER)` → `managers/meetings/llm_call.rs:resolve_provider`. Lokal = `managers::llm::is_local` (streng, `receiver_is_local`), `pinned_local`. | **anschließen** (Projektzweig) |
| W2 | Sprachgespräch, Stufe 2 (Orchestrator, Sprecher/Denker) | `voice_call/turn.rs:voice_call_turn` | Werkzeugergebnisse in den Nachrichten (Notizbuch, Wissen). Ausgeführt werden die Werkzeuge vom Frontend über `commands/voice_tools.rs:voice_tool_call`; `assistent::tor::orchestrator_tor`/`StromTor` lassen gültige Marker durch. | wie W1 (IPC) | `model_id` und `stufe` **aus IPC**, aufgelöst gegen `llm_models` in `turn.rs:resolve`, dazu `sovereignty_allows` | **anschließen**; Werkzeuge im Projekt aus |
| W3 | Recherche (Recherche-Bereich des Projekts, Chat-Spalte der Besprechung, Sprachgespräch über `SprachRecherche.tsx`) | `projekte/befehle.rs:recherche_starten` → `projekte/recherche.rs:RechercheDienst::starten_fuer` → `lauf` → `wai_recherche::recherchiere_schnell`; Fortsetzen: `freigabe_entscheiden` | Nur die Frage. `stuecke` enthält höchstens ein leeres Stück mit Art.-9-Kennzeichen. Die Klasse ist `max(Aufruferklasse, Projektklasse)`. | Halt im Speicher (`Offen`, 15 min) mit gespeichertem `RechercheKontext` | `zustand.rs:AppModelle` → `resolve_provider`, `is_local`; Cloud nur bis INTERNAL (`modell_erlaubt`) | **anschließen** (Projektkontext als `stuecke`) |
| W4a | KI-Dokument im Projekt | — (gibt es nicht) | — | — | — | **neu**: `projekt_ki_dokument_erstellen` → Entwurf in der Ablage |
| W4b | KI-Dokumente Buch, Besprechung, Projektprotokoll | `book_ai/mod.rs:book_ai_summarize`, `commands/meeting_documents.rs:meetings_generate_document`, `commands/project_minutes.rs:project_minutes_generate` | Eigene Quelle (Buch, Transkript, Besprechungsordner), **kein** Projektkontext | Cache/Store je Quelle | `resolve_provider` je Funktion | bleibt; Allowlist-Test hält sie vom Projektkontext fern |
| W4c | Ablage-Entwurf | `ablage/befehle.rs:ablage_entwurf_anlegen` → `ablage/ki.rs:entwurf_anlegen` | Inhalt kommt vom Aufrufer, `mindestklasse` **per IPC** | Journal | — | Projektweg ruft `entwurf_anlegen` nur aus Rust, mit der Mindestklasse aus dem Tor |
| W5 | Stapel (Mehrfachauswahl, Spec §5.4) | — (P5) | — | — | — | P4c liefert die Einzellauf-API und den Test; die Oberfläche kommt mit P5 (Entscheidung E5) |
| — | Besprechungs-Chat | `commands/meeting_chat.rs:meeting_chat_ask` → `managers/meetings/chat/controller.rs:ask` | Eigene Suche (Scope Meeting/Global), `pack` wählt die Auszüge nach Budget | Threads im Rust-Speicher (`render_history`) | `resolve_provider_coded` | kein Projektweg; bleibt |
| — | Buchgespräch | `book_ai/talk/mod.rs:book_talk_ask` | Buch-Index, `assistent::tor::vor_modellaufruf` | eigener Store | `FN_BOOK_TALK` | kein Projektweg; bleibt |
| — | Agent, Automationen | `agent/runtime.rs`, `managers/workflows/` | eigene Schritte, kein Projektkontext | — | eigen | bleibt; Allowlist |

Befunde aus der Inventur, die den Entwurf bestimmen:

1. **Verlauf aus IPC (W1, W2).** Der Kern sieht den Verlauf nur als Text vom Frontend. Er muss deshalb selbst
   erkennen, welche Antwort aus welchem Projektinhalt stammt (Abschnitt 2.5).
2. **Projekttexte ohne Herkunftsmarke.** `kontext::als_text` grenzt Projekttexte nur mit `<<<inhalt N | titel>>>` ab.
   `ablage::ki::bereinigen` greift nur bei Ablage-Auszügen (`<<<auszug Q…>>>` mit `dokument:`-Marke).
3. **Ziel aus IPC (W2).** `voice_call_turn` erhält `model_id` und `stufe` vom Frontend. Die Auswahl eines
   *konfigurierten* Modells ist zulässig. Ob das Ziel lokal ist und welche Klassen es annehmen darf, muss das Tor
   aus der Konfiguration ableiten.
4. **Drei Lokal-Regeln.** `managers::llm::is_local` erkennt nur den eingebauten Server. `compliance::is_local` und
   `usage::provider_is_local` zählen auch Loopback mit. Für Projektdaten gilt die strenge Regel, wie
   `voice_call::receiver_is_local` (Codex M5: Ein Endpunkt auf localhost kann weiterleiten).
5. **Zwei Klassenleitern.** `wai_projekte::Klasse` (Public, Internal, Personal, Special, Restricted) und
   `ablage::zusammenfuehren::Klasse` (Public, Internal, Confidential, Personal, Unbekannt). `KontextText` trägt heute
   keine Klasse.
6. **Keine zustandsbehaftete Provider-Sitzung.** `llm_client` arbeitet zustandslos (Chat Completions). Es gibt weder
   `previous_response_id` noch Slots. „Provider-Sitzung bereinigen“ heißt heute deshalb: Der nächste Versand enthält
   nur den bereinigten Verlauf. Für künftige zustandsbehaftete Sitzungen legt das Tor einen Sitzungsschlüssel fest
   (2.6).
7. **Vorhandene Muster, an die das Tor anschließt:**
   - `voice_call` (`CANCELS`, `mark_context`, `bind_then_check`, `revoke_context_requests` mit Ereignis `revoked`)
   - `assistent::schutz` (dauerhafte Provenienz mit gesalzenen Merkmalen, fail-closed, `ENTZOGEN_*`)
   - `recherche.rs:ModellQuelle` (Trait für die Modellauflösung, `AppModelle` in `zustand.rs`)
   - `kontext.json` (atomar, Sperre, Datei beiseitelegen bei Defekt)

## 2 Versandtor `projekte::tor`

### 2.1 Bausteine und Dateien

| Datei | Inhalt |
|---|---|
| `projekte/tor.rs` (neu) | `ProjektTor`, `Versandauftrag`, `Freigabe`, `Versandschein`, `Abbruch`, Budget, Läufe/Widerruf, Klassenregeln |
| `projekte/tor_herkunft.rs` (neu) | Antwortregister: Provenienz je freigegebener Antwort, `projekte/ki-herkunft.json` |
| `projekte/kontext.rs` | Folgende Felder und Funktionen:<ul><li>`KontextText.klasse` (`wai_projekte::Klasse`) und `KontextText.marke` (`eintrag:<id>#<sha256-16>`)</li><li>`als_text` schreibt die Marke in den Blockkopf</li><li>`inhalte_bereinigen` ist das Gegenstück zu `ablage::ki::bereinigen` für `<<<inhalt`-Blöcke</li></ul> |
| `projekte/dienst.rs` | Die Sichtbarkeit von `kontext_fuer_ki` wird `pub(super)`; Aufrufer ist nur `tor.rs`. Neu ist `with_widerruf(Arc<dyn Fn(&str)>)`, aufgerufen nach jeder Änderung, die den Kontext eines Projekts verengt: schalten, Form/Fassung, entfernen, Projekt löschen oder archivieren. |
| `projekte/zustand.rs` | `Dienste.tor`, `AppZiel: ZielQuelle` nach dem Muster `AppModelle`. Der Widerruf des Dienstes führt auf `tor.widerrufen` und `voice_call::revoke_context_requests`. |
| `projekte/befehle.rs` | `projekt_kontext_schalten(…, expected_stand: Option<String>)`. Damit ist die bekannte Grenze aus P4b geschlossen. |
| `ablage/befehle.rs` | `ablage_ki_setzen`, `ablage_cloud_ausnahme_setzen`, Papierkorb und Verschieben rufen danach `tor.widerrufen_ablage()` auf. Das bricht alle laufenden Projektläufe mit Ablage-Marken ab. |

### 2.2 Schnittstelle (Rust)

```rust
// projekte/tor.rs
pub enum Weg { Chat, Sprache, Dokument, Recherche, Stapel }

/// Projektbindung: aus dem Speicher gelesen (Gespräch: conversations.scope.project_id), nie frei aus IPC.
pub struct Bindung { pub projekt: String, pub gespraech: Option<String> /* conv_id | lauf_id */ }

pub struct Nachricht { pub rolle: Rolle /* System|Nutzer|Antwort|Werkzeug */, pub text: String }

pub struct PromptTeile {
    pub system: String,
    pub historie: Vec<Nachricht>,     // Verlauf (IPC oder Speicher)
    pub zwischenstaende: Vec<String>, // Zusammenfassungen, Denker-Ausgabe
    pub retrieval: Vec<String>,       // fremde Treffer: im Projekt nur leer zulässig
    pub frage: String,
}

pub struct Versandauftrag<'a> {
    pub request_id: &'a str,
    pub bindung: Bindung,
    pub weg: Weg,
    pub funktion: &'a str,            // FN_VOICE_SPEAKER, …
    pub modell: Option<&'a str>,      // nur Auswahl einer Kennung aus llm_models
    pub erwarteter_stand: Option<&'a str>,
    pub teile: PromptTeile,
}

pub struct Freigabe {
    pub schein: Versandschein,        // !Clone, genau einmal einzulösen
    pub nachrichten: Vec<Nachricht>,  // bereinigt, Kontext in der letzten Nutzernachricht
    pub ziel: Ziel,
    pub budget: Budgetbeleg,          // kontext, historie, auszuege, ausgabe, fenster
    pub entzogen: u32,                // ersetzte Antworten/Auszüge
    pub zurueckgesetzt: bool,         // Verlauf verworfen (neue Runde)
    pub abbruch: Arc<tokio::sync::Notify>,
    pub sitzung: String,              // hash(gespraech, stand), für künftige Provider-Sitzungen
}

pub struct Ziel {
    pub anbieter: crate::ablage::ki::ZielAnbieter, // id, lokal (streng), datenklasse_max
    pub provider: PostProcessProvider, pub modell: String, pub schluessel: Option<String>,
    pub pinned_local: Option<String>, pub fenster: u32, pub ausgabe: u32,
}

pub enum Abbruch {
    Gesperrt, ProjektUngueltig, StandVeraltet, Budget(Budgetbeleg), Klasse { code: &'static str },
    ZielUnklar { code: &'static str }, Fremdkontext, RegisterDefekt, Verspaetet, Widerrufen,
}
impl Abbruch { pub fn code(&self) -> &'static str /* "projekt.budget", … */ }

#[async_trait] pub trait ZielQuelle: Send + Sync {
    async fn ziel(&self, funktion: &str, modell: Option<&str>) -> Result<Ziel, Abbruch>;
}
#[async_trait] pub trait KontextQuelle: Send + Sync {   // Produktion: ProjektDienst::kontext_fuer_ki
    async fn kontext(&self, projekt: &str) -> Res<Kontext>;
    async fn projekt(&self, projekt: &str) -> Res<SuiteProjekt>;
}

impl ProjektTor {
    pub async fn vor_versand(&self, a: Versandauftrag<'_>) -> Result<Freigabe, Abbruch>;
    pub async fn abschliessen(&self, s: Versandschein, antwort: &str) -> Result<(), Abbruch>;
    pub fn verwerfen(&self, s: Versandschein);
    pub fn widerrufen(&self, projekt: &str) -> Vec<String>;   // abgebrochene request_ids
    pub fn widerrufen_ablage(&self) -> Vec<String>;
    pub fn mindestklasse(&self, b: &Bindung) -> Result<ablage::Klasse, Abbruch>;
    pub fn gespraech_vergessen(&self, gespraech: &str);
}
```

`vor_versand` ist die Funktion, die der Auftrag als `vor_versand(projekt, stand, ziel, prompt_teile)` skizziert.
Zwei Abweichungen sind Absicht:

- Das Ziel ist kein Parameter, das Tor berechnet es.
- `stand` ist nur eine optionale Erwartung, das Tor liest ihn selbst frisch.

### 2.3 Ablauf `vor_versand` (Reihenfolge ist Teil des Vertrags)

1. **Lauf eintragen**, bevor geprüft wird. Das Tor legt `request_id → (projekt, Notify, veraltet=false)` an, damit
   ein gleichzeitiger `widerrufen` den Lauf trifft (Muster `voice_call`: `CANCELS` vor der Prüfung).
2. **Sperre und Bindung.**
   - Die Sperre ist `PROJEKT_KI_FREIGEGEBEN`; bis P4c-4 kommt `Abbruch::Gesperrt`.
   - Das Projekt muss existieren und darf nicht archiviert sein.
   - Die Bindung stammt aus dem Speicher.
3. **Ziel aus der Konfiguration** (`ZielQuelle`, Produktion `AppZiel`):
   - Den Provider bestimmen `settings.for_function(funktion)` und `llm_models`. Eine `modell`-Kennung wählt nur unter
     den konfigurierten, aktivierten Modellen.
   - `lokal` gilt nur bei `managers::llm::is_local` (eingebauter Server).
   - Bei `compliance::assess(...)` mit `Blocked` bricht das Tor mit `ZielUnklar("projekt.ziel_compliance")` ab.
   - `datenklasse_max` folgt E3.
   - Ist das Ziel lokal, wird das Modell vor dem Kontextlesen mit `ensure_local` gestartet; die Adresse wird
     `pinned_local` (Codex Must-fix 4: Zwischen Prüfung und Versand veraltet nichts).
   - Das Fenster ist `max_input_tokens` oder `context_limit`; lokal das Minimum davon und
     `managers::llm::active_context_tokens`. Die Ausgabereserve ist `max_output_tokens` oder `AUSGABE_VORGABE`
     (Startwert 1 024).
   - Stammt `stufe` aus IPC (W2), gilt das Strengere aus Stufe und Tor. Die Stufe kann nichts erweitern.
4. **Kontext frisch.**
   - `kontext_fuer_ki(projekt)` liefert `stand`.
   - Weicht `erwarteter_stand` ab, folgt `StandVeraltet`. Die Oberfläche zeigt dann die neue Auswahl.
5. **Klassen.**
   - Je Text gilt `max(Eintragsklasse, Projektklasse)`; mit `special_category` gilt `Special`.
   - Klasse > Internal ist nur an ein lokales Ziel erlaubt. Sonst kommt `Abbruch::Klasse("projekt.klasse_ziel")`.
     Das Tor lässt keinen Eintrag still weg; die Oberfläche bietet „lokales Modell wählen“ oder „Eintrag ausschalten“.
6. **Ablage-Auszüge.** Ist `kontext.ablage` nicht leer, ruft das Tor
   `ablage::ki::auszuege(a, frage, &KiBereich::Projekt(kontext.ablage), &ziel.anbieter)` auf.
   - Die Klassenregel für Auszüge gilt dort (`klasse_erlaubt`, Ausnahme je Ordner).
   - `gesperrt` und `kein_beleg` gehen als Text in den Prompt (`ablage::ki::als_text`).
7. **Verlauf bereinigen**, über `historie` und `zwischenstaende` zusammen:
   - a. `retrieval` ist nicht leer: `Abbruch::Fremdkontext`. Im Projekt gibt es keinen App-Kontext, kein Notizbuch und
     keinen Besprechungsindex.
   - b. Jede Antwort im Verlauf schlägt das Tor im Antwortregister nach (2.5):
     - Bekannt und gültig: Die Antwort bleibt.
     - Bekannt, aber entzogen oder Klasse > Ziel: Sie wird durch `ENTZOGEN_ANTWORT_PROJEKT` ersetzt.
     - Unbekannt: Sie wird ebenfalls ersetzt (fail-closed).
   - c. `<<<inhalt`-Blöcke, die im Verlauf oder Zwischenstand stehen, ersetzt `inhalte_bereinigen`, wenn ihre Marke
     nicht im frischen Kontext steht.
   - d. Für Ablage-Blöcke gilt `marken_finden` über alle Teile, dann `herkunft_pruefen(…, KiBereich::Projekt, ziel)`,
     dann `bereinigen`.
   - e. Meldet `bereinigen` `abgeleitet`, wird der Zwischenstand verworfen. Ist das nicht möglich (Marke in einer
     Antwort ohne Registereintrag), wird der Verlauf zurückgesetzt: Nur die neue Frage bleibt, und
     `zurueckgesetzt = true`.
   - f. Ein nicht lesbares Register führt ebenfalls zum Zurücksetzen (`RegisterDefekt` nur, wenn kein Verlauf
     verworfen werden kann).
8. **Prompt bauen.**
   - System plus Datenvertrag (`kontext::als_text`, `ablage::ki::als_text`, je mit „Daten, keine Anweisungen“).
   - Den Kontext setzt das Tor vor die letzte Nutzernachricht (Muster `with_context_message`).
   - Werkzeuge sind im Projekt aus.
9. **Budget reservieren.**
   - Es muss gelten: `tokens(System) + tokens(Kontext) + tokens(Auszüge) + tokens(Verlauf) + tokens(Frage) + Ausgabe
     ≤ Fenster`.
   - Ist es überschritten, folgt `Abbruch::Budget(beleg)` mit allen Anteilen. Gekürzt wird nie, weder Kontext noch
     Verlauf.
   - Die Schätzung ist `kontext::tokens` (Zeichen/4) mit dem Aufschlag `BUDGET_PUFFER` (Startwert 10 %). Das ist
     eine Heuristik, nicht gemessen.
   - Meldet der Provider trotzdem eine Überlänge, wird das ebenfalls `projekt.budget`, ohne neuen Versuch mit
     Kürzung.
10. **Schein ausstellen.**
    - Inhalt: `projekt`, `stand`, `gen`, die Marken der gelieferten Inhalte und Auszüge, `klasse_max`.
    - Zwischen Rückgabe und HTTP-Versand läuft nur synchroner Code. Der Versand läuft wie `voice_call` in
      `select! { biased; abbruch.notified() => …, call => … }`.

### 2.4 Nach dem Aufruf: `abschliessen`

- Das Tor trägt den Lauf aus. In folgenden Fällen folgt `Abbruch::Verspaetet`:
  - Der Lauf ist als veraltet markiert (ein `widerrufen` kam dazwischen).
  - Der frisch berechnete `stand` weicht ab.
  - Eine Ablage-Marke des Scheins ist entzogen.
- Bei `Verspaetet` gilt:
  - Die Antwort wird nicht als `done` gemeldet. Die Oberfläche erhält `revoked`, nimmt den gestreamten Text weg und
    stoppt die Sprachausgabe (vorhandene Auflage 10).
  - Die Antwort wird nicht ins Register eingetragen, nicht gespeichert und nicht wieder eingespeist.
- Sonst trägt das Tor die **freigegebene** Antwort (nach Belegregel und Ausgabetor) ins Register ein, mit den
  Provenienzmarken des Scheins.
- Mit `verwerfen` (Abbruch, Fehler) wird nur ausgetragen, ohne Registereintrag.
- Strenge Regel: Jede Änderung von `stand` verwirft, auch eine reine Inhaltsänderung eines eingeschalteten Eintrags.
  Das ist konservativ und entspricht wörtlich Auflage „verspätete Ergebnisse mit altem `stand`“.

### 2.5 Antwortregister (Widerruf im Verlauf)

- **Datei** `projekte/ki-herkunft.json`, atomar (Temp und Umbenennen) unter Prozess-Sperre, wie `kontext.json`.
  - Fehlend heißt leer.
  - Bei einem Defekt wird die Datei als `ki-herkunft.json.defekt-<ms>` beiseitegelegt, nie gelöscht.
  - Alle Projektgespräche gelten dann als unbekannt, und der nächste Turn setzt zurück.
- **Eintrag**:
  - `gespraech`
  - `merkmal = HMAC-SHA256(salz, normalisierter Antworttext)` (Salz je Datei, wie `assistent::schutz`)
  - `projekt`, `stand`
  - `inhalte: [eintrag:<id>#<h16>]`, `ablage: [dokument:…]`
  - `klasse_projekt`, `klasse_ablage`, `zeit`
  - Nie Klartext, Titel oder Namen.
- **Gültig** ist eine Antwort, wenn alle drei Bedingungen erfüllt sind:
  - Jede `inhalte`-Marke steht im frischen Kontext; das heißt: an, lesbar, gleiche Form, Fassung und gleicher Inhalt.
  - Jede Ablage-Marke ist laut `herkunft_pruefen` berechtigt.
  - Die Klasse ist ≤ dem aktuellen Ziel. Ein Wechsel von lokal zu Cloud mitten im Gespräch ersetzt also Antworten
    aus persönlichen Inhalten.
- **Lokale Anzeige**: Gespeicherte Beiträge in `conversations.db` bleiben stehen. Sie bekommen den Vermerk
  `ext.lva_entzogen = true` (Anzeige grau, „beruhte auf ausgeschaltetem Inhalt“) und gehen nie mehr an ein Modell.
  Der Widerruf betrifft den Versand. Lokale Daten der Person löscht er nicht.
- **Aufräumen**: `gespraech_vergessen` beim Löschen eines Gesprächs. Beim Löschen eines Projekts werden dessen
  Einträge entfernt.

### 2.6 Widerruf laufender Aufrufe und Provider-Sitzung

- Folgende Ereignisse rufen `ProjektTor::widerrufen(projekt)` auf:
  - `kontext_schalten` (auch Form/Fassung)
  - `seite_entfernen`, `buch_entfernen`, `besprechung_entfernen`, `datei_entfernen`
  - `projekt_loeschen`, Archivieren
- Ablage-Freigabeänderungen führen auf `widerrufen_ablage()`. Beide Aufrufe:
  - lösen `Notify` aus und markieren die Läufe als veraltet,
  - lösen `voice_call::revoke_context_requests` aus (Ereignis `revoked`).
- Eine Änderung durch Sync (anderes Gerät) löst keinen Sofortabbruch aus. Sie wird spätestens in `abschliessen`
  erkannt (frischer `stand`, `herkunft_pruefen`). Diese Grenze wird dokumentiert.
- **Provider-Sitzung:** Heute gibt es keine (Befund 6). Für die Zukunft gilt:
  - Eine Sitzung beim Anbieter (Responses-API, Realtime, Slot) darf nur unter `Freigabe.sitzung` geführt werden.
  - Ändert sich `stand` oder setzt das Tor zurück, wird sie verworfen und neu aufgebaut.
  - Ein Test hält fest, dass sich `sitzung` mit `stand` ändert.

### 2.7 Mindestklasse für Entwürfe

`mindestklasse(bindung)` ist das Maximum über alle im Gespräch oder Lauf gelieferten Inhalte und Auszüge. Grundlage
sind die Registereinträge und offenen Scheine, nicht die vom Modell genannten `quellen`. Das Tor bildet die
Projektklasse auf die Ablage ab:

| Projektklasse | Ablage-Klasse |
|---|---|
| Public | Public |
| Internal | Internal |
| Personal, Special, Restricted | Personal |
| unbekannt | `Unbekannt` (dann legt `entwurf_anlegen` nichts an) |

Der Projektweg ruft `ablage::ki::entwurf_anlegen` nur aus Rust auf. Die IPC-`mindestklasse` des allgemeinen Befehls
gilt dort nicht.

### 2.8 Recherche mit Projektkontext

- Projektregeln wie heute (`projektregeln`): Ohne `web_research_enabled` gilt `web_gesperrt`, auch mit Kontext.
  Archiviert heißt abgelehnt.
- `LaufParam` erhält `freigabe: Option<Freigabe>`. Die Texte der Freigabe werden zu `Kontextstueck`; je Stück gilt
  die Klasse aus 2.3 Schritt 5 (`daten_von_klasse`), Art.-9 aus `special_category`.
- Mit Projektkontext hält die Recherche **immer** vor dem ersten Webzugriff an (E4). Die Person sieht die
  Suchanfragen, die aus dem Kontext entstanden sind.
- `Offen` speichert den Versandschein. `freigabe_entscheiden` prüft vor dem Fortsetzen erneut wie `abschliessen`
  (frischer `stand`, Marken). Bei einer Abweichung folgt `kontext_geaendert`, und es wird nichts gesendet.
  `widerrufen(projekt)` räumt die offenen Halte dieses Projekts.
- Das Modell ist das aus `AppZiel`. Es gilt die strengere Regel aus `modell_erlaubt` und 2.3 Schritt 5.

### 2.9 Fehlerfälle

| Code | Wann | Sichtbar als |
|---|---|---|
| `projekt.ki_gesperrt` | Sperre aktiv | „KI im Projekt noch nicht freigegeben“ |
| `projekt.stand_veraltet` | Auswahl hat sich seit der Anzeige geändert | Neu laden, Hinweis |
| `projekt.budget` | Reservierung > Fenster | Banner mit Anteilen; „Einträge ausschalten“, „Gespräch neu beginnen“ |
| `projekt.klasse_ziel` | Klasse zu hoch für das Ziel | „Lokales Modell wählen oder Eintrag ausschalten“ |
| `projekt.ziel_compliance`, `projekt.kein_modell` | Kein zulässiges Ziel | Verweis auf die Einstellungen |
| `projekt.fremdkontext` | App-Kontext, Notizbuch oder Treffer im Projektweg | Programmierfehler; Test |
| `projekt.verspaetet`, `projekt.widerrufen` | Rennen beim Widerruf | Antwort entfernt, Hinweis „Auswahl geändert“ |
| `projekt.zurueckgesetzt` (Hinweis, kein Abbruch) | Verlauf verworfen | Hinweis im Gespräch |
| `projekt.werkzeuge_aus` | `voice_tool_call` in einem Projektgespräch | Kein Aufruf |

Für alle Codes gilt: keine Inhalte, Titel oder Namen in Meldung, Log oder Audit. Protokolliert werden nur Code,
Zähler und Lauf-ID.

## 3 Pakete

Reihenfolge: **P4c-1 → (P4c-2, P4c-3) → P4c-4**. P4c-2 und P4c-3 hängen nur von P4c-1 ab. Sie lassen sich parallel
planen, aber nur nacheinander bauen (ein Rust-Build zur Zeit). Rust-Tests laufen mit
`cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib <filter>`.

### P4c-1 Tor-Kern (nur Backend, kein Weg angeschlossen, Sperre bleibt)

- **Dateien:**
  - neu: `projekte/tor.rs`, `projekte/tor_herkunft.rs`, `projekte/tests_p4c.rs`
  - geändert: `projekte/kontext.rs`, `projekte/dienst.rs`, `projekte/zustand.rs`, `projekte/befehle.rs`,
    `projekte/mod.rs`
  - `bindings.ts` nur ergänzt (`expectedStand`); Frontend in `KontextSpalte`/`useProjekte` übergibt den gelesenen
    `stand`
- **Tests** (`projekte::tests_p4c`, Fakes für `ZielQuelle`/`KontextQuelle`, Kontrolltext `KONTROLLTEXT-7f3a`):
  - `kontrolltext_aus_nie_in_freigabe`: Ein ausgeschalteter Eintrag erscheint in keinem Teil der `Freigabe`.
  - `ausschalten_mitten_im_gespraech_ersetzt_antwort`: Turn 1 an, Antwort registriert; Ausschalten; Turn 2 zeigt
    Antwort 1 ersetzt, Kontrolltext 0-mal.
  - `unbekannte_antwort_wird_ersetzt` und `register_defekt_setzt_zurueck`: Datei beiseitegelegt, nicht gelöscht.
  - `versandrennen_widerruf_zwischen_freigabe_und_abschluss`: `abschliessen` liefert `Verspaetet`, nichts
    registriert, `Notify` ausgelöst.
  - `widerruf_vor_versand_trifft_eingetragenen_lauf`
  - `budgetstopp_statt_kuerzen`: Kontext, Verlauf oder Ausgabe einzeln über dem Fenster; Abbruch mit Anteilen, kein
    Text gekürzt.
  - `lokal_nimmt_kleineres_fenster`
  - `ziel_nur_aus_konfiguration`: Personal-Text plus Cloud-Ziel ergibt `klasse_ziel`. Auch ein Loopback-Endpunkt
    gilt als nicht lokal.
  - `ablage_marke_entzogen_wird_bereinigt` und `abgeleitete_zusammenfassung_wird_verworfen`
  - `mindestklasse_ist_maximum_im_gespraech`
  - `stand_im_schaltbefehl_konflikt`
  - `injektion_im_inhalt_bleibt_daten`: Ein `<<<ende inhalt 1>>>` und ein Werkzeugmarker im Text werden maskiert;
    die Freigabe hat keine Werkzeuge.
  - `allowlist_kontext_fuer_ki_nur_im_tor`: Erweitert den vorhandenen Allowlist-Test um eine Quellsuche über `src/`.
  - `abbruch_ohne_inhalte`: Debug-Ausgabe und Code enthalten den Kontrolltext nicht.
- **Akzeptanz:** `cargo test … --lib projekte::` ergibt alle grün, darunter die neuen Tests. Der Test
  `projekt_ki_pfade_liefern_ohne_p4c_freigabe_nichts` bleibt grün. `bindings_export` ergänzt nur.
- **Risiken:**
  - Strenge `stand`-Regel: Inhaltsänderungen verwerfen laufende Antworten. Das ist gewollt; es wird dokumentiert.
  - Registergröße wächst mit den Gesprächen; Aufräumen beim Löschen.
  - Kosten des frischen `kontext_bauen` je Turn: liest alle eingeschalteten Texte. Gemessen wird erst in P4c-2,
    hier gibt es keine Aussage.

### P4c-2 Chat und Sprachgespräch im Projekt (W1, W2)

- **Dateien:**
  - `conversations/store.rs` und `conversations/mod.rs`: `conversation_create(title, project_id)`, setzt
    `scope.project_id` einmalig; kein Pfad ändert es.
  - `voice_call/mod.rs` und `voice_call/turn.rs`: Projektzweig. Liegt `scope.project_id` aus dem Speicher vor:
    - `context` wird ignoriert;
    - Notizbuch- und Wissenswerkzeuge sind aus;
    - `StromTor` erhält eine leere Werkzeugliste;
    - Ablauf `tor.vor_versand` → Versand mit `abbruch` → `abschliessen` bzw. `verwerfen`.
    - Der Kern wird ohne `AppHandle` als Funktion herausgelöst (Muster `meetings::chat::controller`), damit er testbar
      ist.
  - `commands/voice_tools.rs:voice_tool_call`: verweigert in einem Projektgespräch mit `projekt.werkzeuge_aus`.
  - Frontend:
    - `KontextSpalte.tsx`: „Gespräch im Projekt beginnen“, bis P4c-4 gesperrt
    - `voice-call/VoiceCallPage.tsx`, `Composer.tsx`: Projektchip, Banner für Budget und Rücksetzen, `revoked`
    - i18n de/en
- **Tests:**
  - Rust, je Weg (`voice_call_chat`, `voice_call_turn` als Sprecher und Denker):
    - Kontrolltext;
    - Ausschalten mitten im Gespräch (zwei Turns);
    - Versandrennen: Der Fake-Strom wartet an einer Schranke, `widerrufen` folgt. Ergebnis: Rückgabe leer, `revoked`,
      kein Registereintrag, Fake-Modell danach ohne Aufruf;
    - Verlauf: IPC-Verlauf mit einer Antwort aus einem ausgeschalteten Eintrag wird ersetzt;
    - Budgetstopp mit sichtbarem Code;
    - **Injektion ergibt 0 Werkzeugaufrufe**: Projekttext enthält einen gültigen Werkzeugmarker; das Fake-Modell gibt
      ihn wörtlich aus. Kein Marker verlässt Rust, und `voice_tool_call` zählt 0 Ausführungen;
    - `project_id` ist unveränderlich;
    - ein mitgegebener App-Kontext erreicht den Prompt nicht.
  - Playwright `projekte-ki-gespraech.spec.ts` (gemocktes invoke): Chip, Budget-Banner, `revoked` entfernt Text.
- **Akzeptanz:**
  - `cargo test … --lib voice_call::` und `--lib projekte::` grün.
  - `tsc --noEmit` mit 0 Fehlern.
  - eslint/prettier sauber auf berührten Dateien.
  - Playwright-Spec grün bei 390 und 1440 px.
- **Risiken:**
  - Eingriff in zwei heiße Pfade (`voice_call_chat`, `voice_call_turn`) neben den Notizbuch-Schutz. Der Projektzweig
    muss vor `schutz_pruefen` abzweigen, sonst mischen sich die Register.
  - Für die Stimme: Zwischen Widerruf und `revoked` können bereits erzeugte Sätze noch hörbar sein. Die vorhandene
    Auflage 10 stoppt die Wiedergabe.

### P4c-3 Recherche, KI-Dokument, Stapel-Schnittstelle (W3, W4a, W5)

- **Dateien:**
  - `projekte/recherche.rs`: `LaufParam.freigabe`, Stücke aus dem Projektkontext, erzwungener Halt (E4),
    Halt-Neuprüfung, Räumen beim Widerruf.
  - `vendor/wai-recherche`: Option `SchnellOptionen::freigabe_immer` (Vorgabe `false`), vermerkt in `VENDOR.toml`.
    Dazu ein Upstream-Hinweis an wai-sdk.
  - `projekte/befehle.rs`: `recherche_starten(…, mit_kontext: bool)`, neu `projekt_ki_dokument_erstellen(suite_id,
    auftrag, ordner, name)`.
    - Ablauf: Tor → `send_chat_completion_checked` → `abschliessen` →
      `ablage::ki::entwurf_anlegen(mindestklasse = tor.mindestklasse)` → Mitgliedschaft im Projekt als
      **Automation**. Der Stand ist damit aus, die Ausgabe speist sich nicht selbst wieder ein.
  - `projekte/tor.rs`: `einzellauf` für Stapel. Je Element gibt es einen eigenen Schein und ein eigenes Budget. Nach
    einem Widerruf brechen die restlichen Elemente ab; fertige bleiben stehen.
  - Frontend: `RechercheKarte`, `ChatRecherche` mit Schalter „mit Projektkontext“; Dialog „KI-Dokument erstellen“.
    Beides bis P4c-4 gesperrt.
- **Tests:**
  - Recherche:
    - Kontrolltext aus erscheint weder in `stuecke` noch in Suchanfragen;
    - Halt immer mit Kontext;
    - Ausschalten zwischen Halt und Freigabe ergibt `kontext_geaendert`; der Fake-Suchanbieter zählt 0 Abrufe;
    - ohne Webfreigabe gilt `web_gesperrt` trotz Kontext;
    - Personal plus Cloud wird abgewiesen;
    - Injektion im Kontext erzeugt vor der Freigabe 0 Abrufe.
  - KI-Dokument:
    - Mindestklasse = Maximum, auch wenn das Modell Quellen auslässt;
    - IPC kann sie nicht senken;
    - `Verspaetet` legt keinen Entwurf an;
    - der Entwurf ist im Projekt aus.
  - Stapel: drei Läufe, Ausschalten nach Lauf 2 bricht Lauf 3 ab; Budgetstopp je Lauf.
- **Akzeptanz:** `cargo test … --lib projekte::` und `--lib ablage::` grün; Recherche-Specs in Playwright grün.
- **Risiken:**
  - Vendor-Änderung an `wai-recherche`, mit Abstimmung wai-sdk.
  - Das Ausgabeziel des KI-Dokuments überschneidet sich mit P5 (Ausgabeziel). P4c legt nur den Entwurf und die
    Mitgliedschaft an, ohne Zielwahl.

### P4c-4 Freischaltung (letztes Paket)

- **Dateien:**
  - `projekte/dienst.rs`: `PROJEKT_KI_FREIGEGEBEN = true`
  - `projekte/tests_p4.rs`: Den Sperrtest ersetzt ein Test „ohne Tor kein Kontext“; die Allowlist umfasst alle Wege.
    Dazu ein Quelltest: Im Projektzweig steht `vor_versand` vor jedem `send_chat_completion*`.
  - Frontend: Sperrtexte entfernt (`projektBereich.ki.gesperrt`), Einstiege aktiv.
  - Plan-Ergebnis in `apps/local-voice/docs/plans/2026-11-08_projekte-p4.md`.
- **Tests:** Abnahme Spec §9 „Kontext“ je Weg (Chat, Sprache, KI-Dokument, Recherche, Stapel-API):
  Kontrolltext im ausgeschalteten Eintrag, auch nach dem Ausschalten mitten im Gespräch; fehlendes Kurzbuch liefert
  keinen Rohtext. Dazu Playwright-E2E über die Projektseite.
- **Akzeptanz:**
  - `cargo test … --lib projekte:: voice_call:: ablage::` grün.
  - Windows-Tor grün.
  - Codex-Abnahme JA. Erst danach wird gemergt.
- **Risiken:** Die produktive Projekt-KI ist ab hier aktiv. Ein Rückbau ist ein Einzeiler (Konstante), die Daten
  bleiben unverändert.

## 4 Offene Entscheidungen (mit Empfehlung)

| Nr. | Frage | Empfehlung |
|---|---|---|
| E1 | Welcher Träger für „Chat im Projekt“? | **Sprachgespräch-Unterhaltung** (`voice_call_*` und `conversations.db` mit `scope.project_id`) in Text- und Sprachmodus. Begründung: Ein Verlauf, ein Widerrufsweg; P4b sagt schon „Chat und Sprachgespräch nutzen diese Auswahl“. Kein eigener Projekt-Chat-Befehl, kein `ChatScope::Projekt` im Besprechungs-Chat. |
| E2 | Woher kennt das Tor die Herkunft alter Antworten? | **Antwortregister** mit gesalzenem Merkmal je freigegebener Antwort; unbekannte Antworten werden ersetzt. Es arbeitet mit dem IPC-Verlauf, ohne die Gesprächsführung umzubauen, und folgt dem Muster `assistent::schutz`. Die Alternative (Rust hält den Verlauf selbst) wäre sauberer, ist aber ein Umbau des Orchestrators. Sie wäre ein Folgepaket. |
| E3 | Welche Klasse darf ein Cloud-Ziel annehmen? | Vorgabe `datenklasse_max = Internal` (wie Recherche `modell_erlaubt`). `Confidential` nur, wenn `compliance::assess` unter Profil EU mit bekannten Angaben `Allowed` meldet; dann greifen die Cloud-Ausnahmen je Ablage-Ordner. Projektinhalte ab Personal oder mit Art.-9-Kennzeichen nur lokal. |
| E4 | Recherche mit Projektkontext: Halt immer? | **Ja.** Suchanfragen entstehen aus dem Kontext und verlassen das Haus. Darum hält die Recherche vor jedem Webzugriff an, unabhängig von der Klasse. Umsetzung über die Option `freigabe_immer` im vendorten `wai-recherche`. |
| E5 | Stapel in P4c oder P5? | **P5** baut die Oberfläche der Mehrfachaktionen. P4c liefert `einzellauf` samt Tests und die Allowlist-Pflicht. Die Freischaltung in P4c-4 gilt damit für Chat, Sprache, KI-Dokument und Recherche; Stapel ist ohne eigenen Einstieg nicht erreichbar. |
