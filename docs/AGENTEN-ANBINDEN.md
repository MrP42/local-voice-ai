# Agenten anbinden: MCP und `ctl`

Externe Programme (Claude Code, Codex, Claude Desktop, eigene Skripte) können Local Voice AI auf
zwei Wegen benutzen: **lesend** über den lokalen MCP-Server (Besprechungen suchen und lesen) und
**schreibend** über die laufende App (Datei transkribieren, YouTube-Link anlegen, Vorlesen,
Aufnahme, Sessions). Schreiben geht nie direkt in die Datenbank, sondern immer durch die App:
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
```

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
- Eine Freigabe für `stop_recording` zeigt kein Einwilligungsfenster (sie beendet nur), erscheint
  aber in der Liste der Freigaben auf der Seite Integrationen.

## 8. Fehlersuche

| Beobachtung | Ursache und Abhilfe |
|---|---|
| `ctl` Exit 2, im MCP „Local Voice AI läuft nicht“ | App starten. Die Pipe heißt `\\.\pipe\local-voice-ai-agent-<Benutzerkennung>` (auf der Seite Integrationen steht sie). |
| `ctl` Exit 4, im MCP „Der Zugang wurde nicht angenommen“ | Schlüssel falsch oder Zugang zurückgezogen: neuen Zugang anlegen, `LVA_AGENT_TOKEN` prüfen. |
| `ctl` Exit 3, im MCP „Nicht erlaubt“ | Werkzeug für diesen Zugang oder die Obergrenze auf „Aus“; die Seite Integrationen nennt den Grund. |
| Werkzeug fehlt in der Liste | Aus (Recht), Schalter „Lokaler MCP-Server“ aus, App läuft nicht oder Zugang zurückgezogen. |
| `pending` ohne Ende | Hinweis „Freigaben warten“ auf der Seite Integrationen bzw. das Aufnahme-Fenster beantworten. |

## 9. Selbsttest

`python scripts/mcp_smoke.py --write [<Pfad zur EXE>]` startet eine unsichtbare Sandbox-Instanz
der App (`--agent-bridge-serve`; eigener Ordner, eigene Pipe, kein Fenster, kein Mikrofon, kein
Netz: YouTube-Metadaten kommen aus einer lokalen Attrappe, die Sprach-Engine ersetzt ein Testton)
und prüft den ganzen Weg über den MCP-Proxy: Datei → `meeting_id`, Vorlesen → WAV, YouTube-Link →
Besprechung, `pending` als Hinweis, Aufnahme nie ohne Einwilligung, Protokollversion 2026-07-28,
`ctl`-Exit-Codes (0, 2, 3) und `--audit-dump`. Exit 0 = alles wie erwartet.
