# Agenten anbinden: MCP und `ctl`

Externe Programme (Claude Code, Codex, Claude Desktop, eigene Skripte) können Local Voice AI auf
zwei Wegen benutzen: **lesend** über den lokalen MCP-Server (Besprechungen suchen und lesen) und
**schreibend** über die laufende App (Datei transkribieren, YouTube-Link anlegen, Vorlesen,
Aufnahme, Sessions, **Abläufe der Automationen starten und ihr Protokoll lesen**). Schreiben geht
nie direkt in die Datenbank, sondern immer durch die App:
sie prüft das Recht, fragt dich bei „Fragen“, schreibt jede Aktion ins Protokoll und führt erst
dann aus.

```
Agent  --stdio-->  local-voice-ai.exe --mcp  --Named Pipe (nur du)-->  laufende App
                   (Proxy: übersetzt MCP)                              Zugang prüfen -> Recht je Werkzeug
Skript --Prozess-> local-voice-ai.exe ctl ..-->  (dieselbe Pipe)       -> Freigabe in der App -> Werkzeug -> Protokoll
```

## 1. Zugang anlegen

1. In Local Voice AI die Seite **Integrationen** öffnen und die Integration **Externe Agenten**
   (Art „Agent“) aufmachen.
2. **Zugang anlegen**, einen Namen geben (z. B. „Claude Code“). Der **Schlüssel** (`lvat_…`)
   erscheint **genau einmal**: sofort kopieren. Gespeichert wird nur sein Fingerabdruck.
3. Je Werkzeug festlegen: **Aus** (Standard), **Fragen** (du entscheidest in der App jedes Mal)
   oder **Erlaubt**. Die Rechte-Matrix der Integration („Externer Agent“) ist die Obergrenze: das
   Strengere gilt. **Aufnahme starten** kennt nie „Erlaubt“.
4. Ein Zugang lässt sich jederzeit **zurückziehen**; das gilt sofort, auch in einer laufenden
   Verbindung.

## 2. Programm anbinden

Die Seite zeigt unter „Programm anbinden“ die fertigen Befehle mit dem richtigen Pfad. Der
Schlüssel gehört in die Umgebungsvariable `LVA_AGENT_TOKEN` des MCP-Servers, nie in eine Datei
im Projekt und nie in ein Argument (Argumente stehen in der Prozessliste).

```powershell
# Claude Code
claude mcp add --env LVA_AGENT_TOKEN=<TOKEN> local-voice -- "C:\Program Files\Local Voice AI\local-voice-ai.exe" --mcp

# Codex
codex mcp add local-voice --env LVA_AGENT_TOKEN=<TOKEN> -- "C:\Program Files\Local Voice AI\local-voice-ai.exe" --mcp
```

Claude Desktop (`claude_desktop_config.json`) und andere Clients mit JSON-Konfiguration:

```json
{
  "mcpServers": {
    "local-voice": {
      "command": "C:\\Program Files\\Local Voice AI\\local-voice-ai.exe",
      "args": ["--mcp"],
      "env": { "LVA_AGENT_TOKEN": "<TOKEN>" }
    }
  }
}
```

Ohne `LVA_AGENT_TOKEN` bleibt der Server rein lesend (wie bisher). Die lesenden Werkzeuge brauchen
außerdem den Schalter **Lokaler MCP-Server** (Seite Integrationen); ist er aus, antwortet jedes
Werkzeug mit einem Hinweis, auch die schreibenden.

## 3. Werkzeuge

### Lesend (immer ohne Freigabe, nur wenn der Schalter an ist)

`list_meetings`, `search_meetings`, `get_meeting`, `get_transcript` (nur mit Transkript-Freigabe)
und `get_provenance`: zeigt zu einer fertigen Besprechung, woher Transkript, Protokoll und
KI-Notizen stammen (Modell, Token, Dauer, Zeitpunkt, Quellen, Auslöser, Konfidenz). Es kommen nie
Inhalte, nur Angaben zur Entstehung.

### Schreibend (über die App; Rechte je Werkzeug)

| Werkzeug | Wirkung | Grenzen |
|---|---|---|
| `transcribe_file` | reiht eine Audio-/Videodatei in die Warteschlange ein, liefert `meeting_id` (Status `queued`) | nur absolute lokale Pfade mit Laufwerk (`C:\…`); keine Netzwerkpfade (`\\Server\…`), keine Geräte- oder Namensraumpfade, kein `..`, kein Datenstrom (`:`), keine Gerätenamen (`CON`, `NUL` …); nur Audio-/Videoformate (`wav mp3 m4a aac flac ogg opus wma mp4 mov mkv webm avi`); höchstens 16 GiB; die Warteschlange hat das Speicher-Tor |
| `create_session` | legt einen Ordner (Session) an, liefert `session_id` | Name bis 200 Zeichen |
| `create_meeting` | legt eine leere Besprechung an, auf Wunsch in einer Session | Titel bis 200 Zeichen |
| `add_youtube_source` | Besprechung mit Quelle YouTube für einen **einzelnen** Video-Link | nur ein oEmbed-Abruf (Titel, Kanal); **kein Download**, kein `yt-dlp`; keine Playlists |
| `tts_page_create` | Seite der Vorlesen-Bibliothek mit Text, liefert `page_id` | Text bis 20 000 Zeichen |
| `tts_render_audio` | erzeugt aus dem Text einer Seite eine Audiodatei im Ordner der Seite (Stimme und Format wie in der App eingestellt, Standard WAV) | ein Lauf zugleich; eine vorhandene Datei wird nie überschrieben; Dauer je nach Länge und Engine |
| `start_recording` | Aufnahme starten | **nie ohne deine Einwilligung in der App** (siehe unten); Ende nach `max_minutes` (Vorgabe 480, höchstens 720) |
| `stop_recording` | laufende Aufnahme beenden | gleiches Recht wie „Aufnahme starten“, braucht also immer eine Freigabe |
| `get_action_status` | Stand einer Freigabe (`pending`, `approved`, `denied`, `expired`, `used`) | nur die eigenen Freigaben |

### Automationen: Abläufe auflisten, starten, Protokoll lesen

Drei Werkzeuge verbinden Agenten mit den Abläufen der Automationen (Seite **Automationen**). Sie haben
eigene Rechte: **Automationen lesen** (`list_workflows`, `get_run`) und **Ablauf starten**
(`run_workflow`); beide stehen wie alle Rechte standardmäßig auf „Aus“.

| Werkzeug | Wirkung | Grenzen |
|---|---|---|
| `list_workflows` | listet die Abläufe: `id`, `name`, `enabled`, `armed` (scharf geschaltet), `can_run_live`, `trigger`, `steps`, `variables` (Name, Art, `required`), `open_runs`, `last_run` | ohne die Definition der Schritte; höchstens 200 Abläufe |
| `run_workflow` | startet einen Ablauf mit der Herkunft **Agent**; Argumente `workflow_id`, `live` (Standard `false`), `vars`, `request_id`; liefert `run_id`, `dry_run`, `created`, `state` | **Standard ist der Trockenlauf**; scharf nur unter den Bedingungen unten; `vars` bis 16 KiB und nur die deklarierten Variablen |
| `get_run` | Protokoll eines Laufs: `run` (Zustand, `dry_run`, Herkunft, Zeiten, Fehler) und `steps` (Aktion, Titel, Zustand, Versuch, Fehler, gekürzte Ausgabe) | **ohne** Auslöserdaten, Definition, Eingaben und Geheimnisse; Texte gekürzt, Adressen auf den Server, Schlüssel wie `token` als `***`; bei einem Trockenlauf ohne die eingesetzten Parameter |

**Wann ein Lauf scharf wird.** `run_workflow` plant standardmäßig nur: jeder Schritt meldet, was er
täte und ob er dürfte, nichts wird geschrieben, gesendet oder aufgenommen. Mit `live: true` startet
ein echter Lauf, aber nur, wenn **alles** gilt:

1. Du hast den Ablauf **eingeschaltet und scharf geschaltet** (Seite Automationen). Sonst antwortet
   das Werkzeug mit einem Fehler und es entsteht **kein** Lauf: ein Agent soll nie glauben, etwas
   sei gelaufen. (`list_workflows` zeigt es vorab: `can_run_live`.)
2. Der Auslöser ist **„Durch einen Agenten starten“** oder **„Von Hand starten“**. Ein Ablauf, der durch ein
   Ereignis startet (Termin, Datei, Zeitplan, Kanal, fertige Besprechung), braucht dessen Daten; ein
   Agent kann ihn nur im Trockenlauf planen lassen.
3. Das Recht des Zugangs erlaubt es: **Erlaubt**, oder **Fragen** und du hast in der App
   freigegeben. Die Freigabe ist an die Argumente gebunden, auch an `live`: eine Freigabe für einen
   scharfen Lauf taugt nicht für einen Trockenlauf und umgekehrt.

**Was ein scharfer Lauf darf.** Die Schritte laufen wie bei jedem Lauf mit den Rechten des
**Ablaufs** (Aufrufer „Workflow“): Freigaben für Mail, Dateien, Webhook usw. und die Einwilligung
zur Aufnahme gelten unverändert; ein Agent kann nie mehr auslösen, als du dem Ablauf ohnehin
erlaubt hast. Eine Aufnahme beginnt auch hier nur nach deinem Häkchen „Alle Beteiligten haben
zugestimmt“. Wer einem Zugang „Ablauf starten“ auf **Erlaubt** stellt, erlaubt ihm damit alle
scharfen Abläufe mit Agenten- oder Handauslöser; **Fragen** ist die vorsichtige Einstellung.

**Wiederholung ohne Doppellauf.** Ohne `request_id` ist jeder Aufruf ein eigener Lauf. Gibt der
Agent eine `request_id` mit (1 bis 64 Zeichen: Buchstaben, Ziffern, `_`, `-`), ergibt dieselbe
Angabe denselben Lauf (`created: false`), auch nach einem Abbruch oder App-Neustart. Der Lauf selbst
wird von der Engine der App ausgeführt, nicht vom Aufruf: `run_workflow` kehrt sofort zurück
(Zustand `queued`), den Fortgang liefert `get_run` (`queued`, `running`, `awaiting_approval`, `done`,
`failed`, `cancelled`; ein Schritt mit Freigabe wartet auf dich in der App).

Alle Argumente werden streng geprüft (unbekannte Felder, falsche Typen, Steuerzeichen und zu
lange Texte werden abgewiesen). Inhalte aus Besprechungen und Videos können Anweisungen enthalten,
die nicht von dir stammen: ein Agent soll sie nie als Befehle behandeln. Deshalb ist Schreibendes
standardmäßig **Fragen** und nie „Erlaubt“ für die Aufnahme.

### „Fragen“ und `pending`

Steht ein Werkzeug auf **Fragen**, legt die App eine Freigabe an (Seite Integrationen, Hinweis
„Freigaben warten“). Entscheidest du innerhalb von 30 Sekunden, läuft das Werkzeug und die
Antwort kommt direkt zurück. Sonst antwortet es mit **`pending`** und einer `approval_id`. Das
ist **kein Fehler**:

1. Den Stand mit `get_action_status` (MCP) bzw. `ctl approval <ID>` abfragen.
2. Nach `approved` **denselben Aufruf mit denselben Argumenten** und zusätzlich `approval_id`
   wiederholen (MCP: Argument `approval_id`; `ctl`: `--approval <ID>`).
3. Eine Freigabe gilt **einmal**, nur für diesen Zugang, dieses Werkzeug und diese Argumente und
   verfällt nach einer Stunde. Die Entscheidung gibt es nur in der App, nie über die Pipe.

### Aufnahme und Einwilligung (§ 201 StGB)

`start_recording` startet **nie von selbst**. Die App zeigt sofort das Hinweisfenster „Ein Agent
möchte aufnehmen“ (derselbe Weg wie bei den Abläufen), mit demselben Häkchen wie bei jeder
Aufnahme: „Alle Beteiligten haben zugestimmt“. Erst nach dem Häkchen und „Aufnahme starten“ läuft
die Aufnahme; „Nicht aufnehmen“ lehnt ab. Wer lieber auf der Seite Integrationen im
Freigabedialog entscheidet, braucht dort dasselbe Häkchen. Läuft schon eine Aufnahme oder fehlt
das Mikrofon, startet nichts und der Agent bekommt den Grund. Jede vom Agenten gestartete
Aufnahme hat ein Ende (Vorgabe 8 Stunden), damit eine vergessene nicht die Platte füllt; du kannst
sie jederzeit in der App beenden.

## 4. Das Kommandozeilenwerkzeug `ctl`

```powershell
set LVA_AGENT_TOKEN=<TOKEN>
local-voice-ai.exe ctl status [--json]                     # läuft die App? (ohne Token möglich)
local-voice-ai.exe ctl tools  [--json]                     # welche Werkzeuge darf dieser Zugang jetzt?
local-voice-ai.exe ctl call transcribe_file --args "{\"path\":\"C:\\Aufnahmen\\a.m4a\"}" [--approval ID] [--json]
local-voice-ai.exe ctl approval <ID> [--json]
local-voice-ai.exe ctl workflow list [--json]                          # Abläufe (list_workflows)
local-voice-ai.exe ctl workflow run <ABLAUF-ID> [--live] [--vars JSON|@datei] [--request-id ID] [--approval ID] [--json]
local-voice-ai.exe ctl workflow get <LAUF-ID> [--json]                 # Laufprotokoll (get_run)
```

`ctl workflow ...` ruft die drei Werkzeuge der Automationen mit denselben Rechten, Freigaben und
Exit-Codes wie `ctl call`. `workflow run` plant ohne `--live` nur (Exit 0, `dry_run: true` im
Ergebnis); `--live` startet einen echten Lauf nur bei scharfem Ablauf (sonst Exit 1 und Hinweis,
kein Lauf). Ohne das Recht „Ablauf starten“ endet `workflow run` mit **Exit 3**.

Exit-Codes: **0** ausgeführt/läuft, **1** Fehler, **2** App nicht erreichbar, **3** nicht erlaubt
(Werkzeug „aus“ oder abgelehnt), **4** Anmeldung fehlgeschlagen, **5** wartet auf deine Freigabe.
Das Token kommt aus `LVA_AGENT_TOKEN` oder `--token-file <Datei>`, nie aus einem Argument.

## 5. Protokollversionen

Der MCP-Proxy spricht drei Fassungen („dual-era“):

- `2025-06-18` und `2025-11-25`: mit `initialize`-Handshake, wie bisher.
- **`2026-07-28`**: zustandslos. Jede Anfrage trägt `_meta` mit `io.modelcontextprotocol/protocolVersion`
  und `…/clientCapabilities`; es gibt keinen Handshake. `server/discover` nennt die Versionen
  (`supportedVersions`), Fähigkeiten und den Namen des Servers. Ergebnisse tragen `resultType:
  "complete"`, `tools/list` und `server/discover` zusätzlich `ttlMs: 0` und `cacheScope: "private"`
  (die Liste hängt von Schalter, Zugang und laufender App ab). Eine unbekannte Version beantwortet
  der Server mit Fehler `-32022` und der Liste der unterstützten Versionen, eine Anfrage ohne
  `clientCapabilities` mit `-32602`.
- Eine Anfrage ganz ohne Versionsangabe wird wie bisher tolerant als alte Fassung bedient.

## 6. Protokoll (Audit)

Jede Aktion eines Agenten, eines Ablaufs und jede Verweigerung steht im Protokoll: in der App auf
der Seite Integrationen und per Kommandozeile:

```powershell
local-voice-ai.exe --audit-dump --json [--audit-limit 1000] [--out audit.json]
```

Der Dump liefert Zeit, Aufrufer, Integration, Fähigkeit, Ziel, Ergebnis und Detail je Aktion
(älteste zuerst) und Summen je Ergebnis, Aufrufer und Fähigkeit über die ganze Tabelle.
**Aufbewahrung gedeckelt:** die Tabelle hält höchstens 20 000 Zeilen; beim Schreiben fallen die
ältesten zuerst, Verweigerungen vor echten Aktionen, und gleiche Verweigerungen werden
zusammengefasst. Der Dump liefert nie mehr als diese Grenze und meldet mit `truncated`, ob mehr
da ist. Kein Geheimnis gelangt ins Protokoll (Kennwörter, Token und Adresspfade werden
geschwärzt). Aus Sicherheitsgründen läuft `--audit-dump` wie `--integrations-dump` nur gegen eine
Sandbox (`LVA_MEETINGS_DIR`), nie gegen die produktive Datenbank.

## 7. Sicherheit und Grenzen

- Die Pipe ist nur für den aktuellen Windows-Benutzer erreichbar (eigene Zugriffsliste, keine
  Fernzugriffe, Prüfung des Gegenübers). Ein Agent mit freiem Shell-Zugriff unter demselben
  Benutzer kann die Datenbank direkt ändern; die Rechte gelten gegenüber Programmen, die über
  Pipe, MCP oder `ctl` sprechen.
- Je Zugang höchstens 60 Aufrufe pro Minute; höchstens 8 gleichzeitige Verbindungen.
- Der Proxy hält keine Verbindung offen: eine neue App-Version, ein zurückgezogener Zugang oder
  ein gewechselter Schalter gilt bei der nächsten Anfrage.
- Läuft die App nicht, fehlen die schreibenden Werkzeuge in der Liste und ein Aufruf antwortet mit
  dem Hinweis, die App zu starten (kein Absturz, keine Wartezeit ohne Ende).
- `tts_render_audio` startet die Sprach-Engine der App über deren Speicher-Tor; bei knappem
  Arbeitsspeicher meldet die App das, statt den Rechner zu belasten. Ein neuer Export in der App
  beendet einen laufenden Lauf des Agenten (und umgekehrt).
- Automationen: `run_workflow` ist je Zugang auf 60 Aufrufe pro Minute begrenzt, je Ablauf warten
  höchstens 500 Läufe. Das Protokoll (`get_run`) gibt keine Auslöserdaten, Definitionen, Eingaben und
  Geheimnisse heraus; wer mehr sehen will, öffnet den Lauf in der App. Ein Agent kann einen Ablauf
  weder anlegen noch ändern, ein- oder scharf schalten und keine Freigabe eines Schritts entscheiden.
- Eine Freigabe für `stop_recording` zeigt kein Einwilligungsfenster (sie beendet nur), erscheint
  aber in der Liste der Freigaben auf der Seite Integrationen.

## 8. Fehlersuche

| Beobachtung | Ursache und Abhilfe |
|---|---|
| `ctl` Exit 2, im MCP „Local Voice AI läuft nicht“ | App starten. Die Pipe heißt `\\.\pipe\local-voice-ai-agent-<Benutzerkennung>` (auf der Seite Integrationen steht sie). |
| `ctl` Exit 4, im MCP „Der Zugang wurde nicht angenommen“ | Schlüssel falsch oder Zugang zurückgezogen: neuen Zugang anlegen, `LVA_AGENT_TOKEN` prüfen. |
| `ctl` Exit 3, im MCP „Nicht erlaubt“ | Werkzeug für diesen Zugang oder die Obergrenze auf „Aus“; die Seite Integrationen nennt den Grund. |
| Werkzeug fehlt in der Liste | Aus (Recht), Schalter „Lokaler MCP-Server“ aus, App läuft nicht oder Zugang zurückgezogen. |
| `run_workflow` mit `live` meldet „nicht scharf geschaltet“ oder „startet durch ein Ereignis“ | Der Ablauf ist nicht eingeschaltet und scharf, oder sein Auslöser ist ein Ereignis: auf der Seite Automationen einschalten, scharf schalten und den Auslöser „Durch einen Agenten starten“ wählen. Ohne `live` läuft immer ein Trockenlauf. |
| `get_run` zeigt `awaiting_approval` | Ein Schritt des Ablaufs wartet auf deine Freigabe (Seite Integrationen) oder, bei einer Aufnahme, auf das Einwilligungsfenster. |
| `pending` ohne Ende | Hinweis „Freigaben warten“ auf der Seite Integrationen bzw. das Aufnahme-Fenster beantworten. |

## 9. Selbsttest

`python scripts/mcp_smoke.py --write [<Pfad zur EXE>]` startet eine unsichtbare Sandbox-Instanz
der App (`--agent-bridge-serve`; eigener Ordner, eigene Pipe, kein Fenster, kein Mikrofon, kein
Netz: YouTube-Metadaten kommen aus einer lokalen Attrappe, die Sprach-Engine ersetzt ein Testton)
und prüft den ganzen Weg über den MCP-Proxy: Datei → `meeting_id`, Vorlesen → WAV, YouTube-Link →
Besprechung, `pending` als Hinweis, Aufnahme nie ohne Einwilligung, Protokollversion 2026-07-28,
`ctl`-Exit-Codes (0, 2, 3) und `--audit-dump`. Exit 0 = alles wie erwartet.

`python scripts/mcp_smoke.py --workflows [<Pfad zur EXE>]` prüft die Automationen auf demselben Weg
(Sandbox-Instanz mit Engine, zwei Abläufe, vier Zugänge: voll, nur lesend, fragen, ohne Recht):
`list_workflows` (Felder, keine Definition), `run_workflow` als Trockenlauf (Herkunft Agent),
`get_run` (Lauf `done`, Schritte mit Plan, ohne eingesetzte Parameter), `live` nur bei scharfem
Ablauf (sonst Fehler und kein Lauf), `request_id` ohne Doppellauf, `ctl workflow list|run|get`
(Exit 0; ohne Recht 3, ohne Token 4, Fragen 5) und der Audit-Dump. Exit 0 = alles wie erwartet.

## 10. n8n anbinden

n8n (selbst gehostet, Community Edition) bleibt eine **Brücke**: die App hat keinen offenen
Netzwerkport, n8n ruft die App also **nicht** per Webhook-Knoten auf. Es gibt zwei Richtungen:

**App → n8n (Webhook hinaus).** Ein Ablauf schickt mit dem Baustein **Webhook senden** Daten an einen
Webhook in n8n.

1. In n8n einen Workflow mit dem Knoten **Webhook** (Methode `POST`) anlegen und seine
   **Production-URL** kopieren (bei lokalem n8n `http://127.0.0.1:5678/webhook/<Kennung>`).
2. In Local Voice AI unter Integrationen **Webhook (n8n)** hinzufügen, Name z. B. „n8n-lokal“, und die
   Adresse eintragen. Sie liegt im Geheimnisspeicher, denn der Schlüssel steckt im Pfad; im Ablauf,
   im Protokoll und in Fehlermeldungen steht nur der Server. Erlaubt sind `https` und `http` nur
   gegen den eigenen Rechner (lokales n8n); Umleitungen werden nie befolgt.
3. Im Ablauf den Baustein „Webhook senden“ mit `via` = Name der Integration und `body` setzen
   (Beispiel: [`beispiele/n8n-app-sendet-an-n8n.lva-workflow.json`](beispiele/n8n-app-sendet-an-n8n.lva-workflow.json),
   über die Seite Automationen importieren). Das Recht „Daten an Webhook senden“ bestimmst du an
   der Integration; Schreibendes fragt standardmäßig nach.

n8n bekommt einen JSON-Körper mit `schema` (`lva-webhook@1`), `workflow` (`id`, `name`), `run`,
`step`, `idempotency_key` und `body`, dazu den Header `Idempotency-Key`. Der Schlüssel (`<Lauf>:<Schritt>`)
bleibt bei einer Wiederholung gleich: n8n kann damit Doppeltes erkennen. Ob die Daten beim Webhook
angekommen sind, ist nach einem Zeitlimit manchmal unklar; die App wiederholt dann nie von selbst.

**n8n → App (über `ctl` oder eine Datei).**

- *n8n läuft direkt unter Windows* (Desktop-App oder npm): Der Knoten **Execute Command** startet
  `local-voice-ai.exe ctl workflow run <ABLAUF-ID> --request-id <Kennung> --token-file <Datei> --json`.
  Dafür einen Zugang anlegen (Abschnitt 1), „Ablauf starten“ auf **Fragen** oder **Erlaubt** stellen
  und den Schlüssel in einer Datei ablegen, die nur dein Benutzer lesen kann. Der Ablauf bekommt den
  Auslöser „Durch einen Agenten starten“ ([`beispiele/n8n-ruft-app-per-ctl.lva-workflow.json`](beispiele/n8n-ruft-app-per-ctl.lva-workflow.json)),
  der n8n-Workflow steht als [`beispiele/n8n-ruft-app-per-ctl.n8n-workflow.json`](beispiele/n8n-ruft-app-per-ctl.n8n-workflow.json)
  bereit (n8n: Workflow importieren; Pfade und `ABLAUF-ID` anpassen). Ohne `--live` ist es ein
  Trockenlauf; Exit 5 heißt: die App wartet auf deine Freigabe. Neuere n8n-Versionen können den
  Knoten „Execute Command“ standardmäßig abgeschaltet haben; das ist eine bewusste
  Einstellung von n8n.
- *n8n läuft im Docker-Container*: Ein Container startet keine Windows-Programme und erreicht die
  Pipe nicht. Hier legt n8n eine Datei in einen Ordner, den die App mit dem Auslöser
  **Datei im Ordner** beobachtet (Integration „Ordner“, z. B. den eingebundenen Austauschordner von
  n8n). Die Datei startet den Ablauf wie jede andere.
- Agenten, die n8n bedienen, sprechen direkt MCP/`ctl` (Abschnitte 2 bis 4); n8n muss dafür nichts tun.

Die Beispiele sind gegen die App geprüft (`--workflow-run <Datei> --dry-run`), nicht gegen eine laufende
n8n-Instanz: das n8n-Beispiel ist ein Ausgangspunkt, dessen Knotenparameter je n8n-Version leicht
abweichen können.
