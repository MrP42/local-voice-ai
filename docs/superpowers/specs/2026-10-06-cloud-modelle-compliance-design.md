# Cloud-Modelle, Regelwerk und Compliance-Schild – Entwurf

Stand 06.10.2026 · Auftrag Patrick · Zweig `feat/cloud-modelle-compliance`

## Ziel

Cloud-Sprachmodelle von Anthropic, OpenAI und Mistral für Protokolle, Notizen, Übersetzung
nutzbar machen – über das **Abo** (Claude Pro/Max, ChatGPT Plus/Pro) **und** per API-Schlüssel.
Jede Wahl wird gegen ein **Regelwerk** (zunächst „EU“) geprüft; ein **Schild** in der Fußleiste
zeigt, ob alles sicher und regelkonform ist, und erklärt es auf Klick.

Entscheidungen (06.10.): Anbindung **beides** · Regelwerk **EU** · Verstoß **sperren + rotes
Schild** · Anbieter zuerst **Anthropic, OpenAI, Mistral**.

## Bestand (gesichtet)

- Verbindungen `LlmConnection` mit Vorlagen (`settings.rs:1258`): openai, anthropic, openrouter,
  groq, cerebras, bedrock_mantle (fest us-east-1), zai, ollama, vllm, local, custom. Alle Aufrufe
  OpenAI-kompatibel (`llm_client.rs`), Budget-Prüfung vor jedem Aufruf (`:284`), Verbrauchsbuch
  `usage.rs` mit Anbieter je Aufruf.
- **API-Schlüssel im Klartext** in `settings_store.json` (`post_process_api_keys`). DPAPI-Ablage
  existiert schon (`managers/calendar/secret.rs`, Namespace `Integration`).
- Kein Abo-Weg, kein Standort, kein Regelwerk.

## Spike 06.10. (Abo über offizielle CLIs)

| Weg | Aufruf | Kontext | Zeit |
|---|---|---|---|
| Claude Pro/Max | `claude -p --output-format json --model sonnet --system-prompt … --tools "" --no-session-persistence --setting-sources "" --strict-mcp-config --mcp-config '{"mcpServers":{}}' --disable-slash-commands`, leerer Arbeitsordner, Prompt über stdin | 784 Tok | 3,4 s |
| ChatGPT | `codex exec --json --skip-git-repo-check --sandbox read-only --ephemeral -m gpt-6-astra -c mcp_servers={} -`, Prompt über stdin, Antwort = letztes `item.completed/agent_message` | 20,7 kTok | 7,9 s |

Ohne Isolation lädt `claude -p` die persönliche Konfiguration (89 kTok, **Hooks laufen mit**).
`--bare` scheidet aus: liest den Abo-Login (OAuth) nicht. Mit ChatGPT-Konto lehnt der Server
`gpt-6.1-sol`/`gpt-6` ab; `gpt-6-astra` geht.

## Bausteine

1. **Abo-Anbieter** `claude_cli`, `codex_cli` (neue Vorlagen, `managers/llm/cli.rs`):
   - Pfad zur CLI automatisch gefunden (PATH, `~/.local/bin`, npm-global), Login-Status per
     Probeaufruf; Modelle als feste Liste (Claude: haiku/sonnet/opus; Codex: die mit
     ChatGPT-Konto zugelassenen).
   - Aufruf wie im Spike, isoliert, ohne Fenster, mit Zeitlimit; Token aus der JSON-Antwort ins
     Verbrauchsbuch (Preis 0 – Abo).
   - Strukturierte Ausgabe: Schema in den Systemprompt, Antwort als JSON geparst (wie bei
     Anbietern ohne `supports_structured_output`). Streaming: Antwort am Stück. Bilder: nicht
     unterstützt → klare Meldung.
2. **Mistral** als API-Vorlage (`https://api.mistral.ai/v1`, OpenAI-kompatibel).
   **OpenAI EU-Datenresidenz** als Variante, wenn die Recherche den Endpunkt bestätigt.
3. **Schlüssel verschlüsselt**: `post_process_api_keys` → DPAPI (`secret.rs`, eigener Slot je
   Verbindung); einmalige Migration, danach Klartext gelöscht. Auf macOS: Schlüsselbund oder,
   solange nicht gebaut, Klartext mit **gelbem Schild** („Schlüssel unverschlüsselt“).
4. **Anbieterfakten** (`managers/compliance/facts.rs`): je Vorlage/Variante Serverland(-länder),
   EU-Residenz, DPF-Zertifizierung, Training mit Kundendaten, AVV, Quelle, Prüfdatum.
   Abo-Pläne getrennt von API (andere Bedingungen). Fakten aus der Recherche vom 06.10., nicht
   geraten; Unbekanntes bleibt „unbekannt“ und zählt nicht als erfüllt.
5. **Regelwerk** (Einstellung `compliance_profile`: `eu` · `local_only` · `none`, Standard `eu`):
   Bewertung je Modell → *erlaubt* · *erlaubt mit Bedingung* (Grund) · *gesperrt* (Grund).
   Durchsetzung an drei Stellen: Aktivieren abgewiesen, Auswahl ausgegraut mit Grund, und
   vor jedem Aufruf in `llm_client` (wie das harte Budget) – Code `compliance_blocked`.
6. **Modellauswahl**: hinter dem Modellnamen Wolkensymbol für Cloud-Modelle und Länderflagge
   des Serverstandorts als **SVG** (Windows zeigt Flaggen-Emoji nur als Buchstaben).
   Fußleisten-Auswahl, Einstellungen, Modellkarten.
7. **Schild** (Fußleiste rechts, vor „Nach Updates suchen“): grün / gelb / rot; Klick öffnet
   eine Erklärung: aktives Regelwerk, aktives Modell und seine Bewertung, Standort, Schlüssel-
   Verschlüsselung, letzte Aufrufe (aus dem Verbrauchsbuch: gesperrte Versuche, Fehler).
   - **Grün**: aktives Modell erlaubt, Schlüssel verschlüsselt, keine Sperre/kein Fehler in 24 h.
   - **Gelb**: erlaubt mit Bedingung, Fakten unbekannt/älter als 180 Tage, Anbieterfehler in 24 h,
     Schlüssel unverschlüsselt.
   - **Rot**: aktives Modell verstößt, gesperrter Aufruf in 24 h, Abo-Login abgelaufen.
8. **Einstellung** im Reiter „Nachbearbeitung“ (AGENTS.md: kein neuer Reiter), Gruppe „Regelwerk“.

## Etappen

1. Fakten + Regelwerk + Durchsetzung + Schild + Symbole (mit bestehenden API-Verbindungen,
   Mistral-Vorlage) – Prototyp, Tests, Installer.
2. Schlüssel-Verschlüsselung mit Migration.
3. Abo-Anbieter `claude_cli`, `codex_cli`.
4. Abnahme-Installer, Release gebündelt.

## Grenzen

Kein Rechtsrat: das Schild prüft dokumentierte Anbieterangaben gegen ein Regelwerk, es ersetzt
keine Datenschutzprüfung. Abo-Nutzung über CLIs nur für den Eigengebrauch, Bedingungen siehe
Recherche (Abschnitt folgt).
