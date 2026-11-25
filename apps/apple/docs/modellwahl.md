# Gemeinsame Modellwahl (iPhone)

Kern: `VoiceCore/Sources/VoiceCore/Models/`. Oberfläche: `iPhone/Models/`. Entscheidung und Pakete: lva#801, Nutzung auch in lva#804.

## Wege

| Route | Wo gerechnet wird | Zustimmung |
|---|---|---|
| `.local(modelId:)` | auf dem iPhone (llama.cpp, Apple Foundation Models) | nein |
| `.endpoint(id:)` | eigener Server im Netz oder Cloud-API (Schlüssel im Schlüsselbund) | ja, je Gegenstand und Zweck |
| `.pcRelay(provider:model:)` | über den gekoppelten PC, auch Abos (Claude Code CLI, Codex CLI) | ja, je Gegenstand und Zweck |

Alles außer `.local` gilt als extern. Der Live-Stand während einer Aufnahme (`ModelPurpose.liveState`) läuft immer lokal.

## Zustand

Die App hält genau einen `ModelAccessState` und pflegt ihn:

- `setPolicy(_:)`: Kundenprofil. Bis es geladen ist, gilt `.unknown` (nur lokal).
- `setTarget(_:for:)`: aufgelöste Route mit Anbieter und Ziel-URL, aus den Endpunkt-Einstellungen bzw. aus der Modellliste, die der PC meldet. `nil` entfernt die Route; danach ist sie verboten.
- `record(_:)`, `revoke(_:)`, `revokeAll(subjectId:)`: Zustimmungen. Gespeichert wird `consentRecords()`.

Jede Änderung erhöht die Version. Für den Zustimmungsdialog liefert `consentScope(route:purpose:subjectId:dataCategories:)` den geprüften Geltungsbereich und das aufgelöste Ziel; beides geht an `ModelConsentSheet`.

## Ausführen

```swift
let session = try await ModelGate.open(provider, purpose: .meetingMinutes, subjectId: meeting.id,
                                       dataCategories: ["Transkript", "Teilnehmernamen"], state: accessState)
for try await event in session.stream(request) { … }
```

Die Sitzung stellt bei jedem Aufruf eine neue, einmal einlösbare Berechtigung aus. Ändert sich der Freigabestand zwischen Ausstellen und Einlösen, stellt sie genau einmal neu aus, solange noch nichts geliefert wurde; sonst endet der Aufruf mit `ModelFailure.stateChanged`.

## Vertrag für Adapter (`ModelProvider`)

1. **`confirm` unmittelbar vor dem Senden.** Nach dem letzten eigenen `await` und direkt vor dem Netz- oder Modellaufruf `try await ModelProviderCheck.confirm(self, request, authorization)`. Eine erfolgreiche Einlösung gibt nur den unmittelbar folgenden Sendestart frei: zwischen `confirm` und dem Start der Anfrage (z. B. `URLSession.bytes(for:)`, `lv_llama_chat`) steht kein weiteres `await`. Einlösung und Sendestart sind nicht atomar; ein Wechsel genau dazwischen wirkt erst beim nächsten Aufruf.
2. **Tatsächliches Ziel prüfen.** Endpunkt-Adapter senden nur, wenn `authorization.permitsSending(to: url)` für die URL gilt, die sie wirklich benutzen (nach allen Umleitungen; Umleitungen auf andere Hosts nicht folgen).
3. **Keine Inhalte in Fehlern und Logs.** Fehler nur als `ModelFailure` bzw. `ModelFailure.provider(ModelProviderError(status:))`, nie mit Antworttext, Prompt oder Schlüssel. Die Sitzung bildet sonstige Fehler zusätzlich über `ModelFailure.sanitized` ab (Abbruch bleibt Abbruch).
4. **Abbruch.** Task-Abbruch beendet den Transport (`onTermination` → Anfrage abbrechen).
5. **Berechtigungen nicht speichern.** Eine Berechtigung gilt für genau einen Aufruf.
