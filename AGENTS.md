# AGENTS.md — Repo-Wurzel

Diese Datei gilt für **alle** KI-Zugänge und Werkzeuge an diesem Repo (Claude Code,
Codex/ChatGPT, opencode, lokal wie in der Cloud, Windows wie macOS). Sie steht
absichtlich in der Wurzel, weil sie die Zusammenarbeit regelt und nicht die App.

Die App selbst ist unter [`apps/local-voice/AGENTS.md`](apps/local-voice/AGENTS.md)
beschrieben (Build, Architektur, Einstellungen, Releases). Wer an der App
arbeitet, liest beide.

## An diesem Repo arbeiten mehrere Entwickler gleichzeitig

Seit dem 05.09.2026 arbeitet mehr als ein Zugang parallel an diesem Repo —
mindestens ein lokaler Windows-Zugang und ein Cloud-/macOS-Zugang. Jede Annahme
„ich bin allein im Baum" ist ab sofort falsch. Der Arbeitsbaum, den du siehst,
ist eine Momentaufnahme; `origin/main` kann sich zwischen zwei deiner Befehle
bewegt haben.

### Verbindlich

1. **Nie direkt auf `main` committen oder pushen.** Zweig anlegen
   (`feat/…`, `fix/…`, `chore/…`, `docs/…`), dort arbeiten, PR öffnen. Auch für
   Einzeiler und auch für Dokumentation.
2. **Nie Historie umschreiben, die gepusht ist.** Kein `push --force` (auch nicht
   `--force-with-lease` auf `main`), kein `rebase` und kein `commit --amend` auf
   bereits gepushten Commits. Auf dem EIGENEN, noch nicht gepushten Zweig ist
   Rebase in Ordnung.
3. **Vor jedem Push `git fetch origin`** und den eigenen Zweig auf den aktuellen
   Stand bringen. Vor jedem Merge zusätzlich `git log --oneline origin/main..HEAD`
   UND `git log --oneline HEAD..origin/main` ansehen — wer nur die eine Richtung
   prüft, merkt fremde Arbeit erst, wenn er sie überschrieben hat.
4. **Konflikte werden gelöst, nicht weggeräumt.** Kein `git checkout --ours/--theirs`
   über einen ganzen Konflikt, kein `reset --hard` auf einen Zweig mit fremden
   Commits, kein `git clean -fdx` im geteilten Baum. Wer eine fremde Änderung nicht
   versteht, fragt nach, statt sie zu entfernen.
5. **`git stash` ist im geteilten Baum verboten, solange ein Build oder Test läuft.**
   Der Baum steht dann kurz auf `HEAD`, und der Build backt die alten Quellen ein.
   (Am 05.09.2026 genau so passiert; der Installer musste neu gebaut werden.)
6. **Nur anfassen, was zur eigenen Aufgabe gehört.** Keine Formatier- oder
   Aufräumläufe über fremde Dateien (`prettier --write .`, `cargo fmt` über den
   ganzen Baum, Massen-Renames) — sie erzeugen Konflikte in jeder parallelen
   Arbeit. Vorbestehende Verstöße bleiben stehen, siehe unten.
7. **Releases nur von aktuellem `main`**, nachdem alle offenen PRs entweder drin
   oder bewusst ausgeklammert sind. Tag-Schema und Ablauf:
   [`apps/local-voice/AGENTS.md`](apps/local-voice/AGENTS.md#versioning-and-releases).
   Ein Release ist ein Ereignis für alle Zugänge — vorher ansagen.
8. **Lange Läufe ansagen.** Wer einen Release-Build oder eine lange Testreihe
   startet, sagt es im PR oder im Auftrag; wer parallel dazu den Baum umbaut,
   zerstört das Ergebnis.

### Warum so streng

Der teure Fehler ist nicht der Konflikt — den zeigt Git an. Teuer sind die
stillen: ein `--force`, das fremde Commits verschwinden lässt; ein `reset --hard`,
das eine halbe Stunde fremder Arbeit wegnimmt; ein Formatierlauf, der jeden
späteren Merge zur Handarbeit macht. Alle drei sehen im Moment des Ausführens
erfolgreich aus.

## Vorbestehendes Rot nicht mit eigener Schuld verwechseln

Auf `main` sind mehrere Prüfungen schon rot, ohne dass eine laufende Änderung
schuld wäre. Wer sie „mitrepariert", erzeugt genau die breiten Diffs, die Regel 6
verbietet:

- `prettier --check` ist auf unberührten Dateien rot (u. a. `TagPalette.tsx`,
  `tagProvider.tsx`; dazu CRLF im Windows-Checkout).
- `cargo clippy` bricht mit `approx_constant` in `settings.rs` ab.
- `pnpm check:translations` läuft ohne installiertes `tsx` gar nicht; zusätzlich
  fehlen ~306 `tts.*`-Schlüssel in 22 Sprachen vorbestehend.

Eigene Änderungen deshalb gezielt prüfen (nur die berührten Dateien) und den
Befund benennen, statt den Baum grün zu machen.
