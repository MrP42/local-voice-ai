# Abo-Vorschau im Buch-Gespräch: Lizenz- und Weitergabe-Bewertung

Stand: 06.11.2026, lva#334, Phase 1 (nur Claude Code). Dies ist eine **Bewertung
auf Grundlage der veröffentlichten Bedingungen, keine Rechtsberatung**. Sie
stellt nicht verbindlich fest, dass eine Nutzung zulässig ist.

## Quellen (abgerufen am 06.11.2026)

Maßgeblich (primär): Anthropic, Claude Code Docs, „Legal and compliance“,
<https://code.claude.com/docs/en/legal-and-compliance>. Wörtliche Kurzzitate:

1. „Anthropic does not permit third-party developers to offer Claude.ai login
   into their own applications, or to route requests through Free, Pro, Max, or
   Ultra plan credentials on behalf of their users. Moreover, developers may not
   collect, store, or intermediate Claude.ai credentials or session tokens —
   sign-in to a Claude account must complete through Anthropic's own flow.“
2. „Nor does it prevent an end user from signing in to the unmodified Claude Code
   binary with their own Claude subscription, including where a platform hosts
   Claude Code as described under ‚Can customers offer Claude Code in their
   products?‘“
3. Abschnitt „Can customers offer Claude Code in their products?“: Claude Code in
   Produkten vorzuinstallieren oder dort auszuführen setzt die Zustimmung zu den
   Commercial Terms voraus. Außerdem: „The Claude Code binary must not be
   modified. Claude Code must be installed and run as published by Anthropic …“
   und „Customers may not pay for, resell, or intermediate Claude usage on their
   end users' behalf. Each end user must authenticate with their own … Claude
   subscription plan credentials …“
4. „Advertised usage limits for Pro, Max, and Ultra plans assume ordinary,
   individual usage of Claude Code and the Agent SDK.“

Sekundär (nur Kontext, nicht maßgeblich): Anthropic hat am 19.02.2026 die
Dokumentation zur OAuth-Nutzung in Drittanwendungen präzisiert (The Register,
20.02.2026,
<https://www.theregister.com/2026/02/20/anthropic_clarifies_ban_third_party_claude_access>);
Drittanbieter-Harnesses wurden ab 2026 gesperrt bzw. abgerechnet.

## Was die App technisch tut

- Sie startet die **vom Nutzer selbst installierte, unveränderte** native
  Claude-Code-Datei lokal auf dessen Rechner, mit **dessen eigenem Login**. Die
  Anmeldung läuft ausschließlich über Anthropics eigenen Ablauf in der CLI.
- Sie **nutzt keine Tokens, liest keine Credentials und legt keine an**. Kein
  `--bare`, damit der OAuth-Login der CLI gilt.
- Sie **liefert die CLI nicht mit** und enthält keinen Teil davon.
- **Keine Binärkopie mehr.** Frühere Entwürfe legten eine lokale Kopie der Datei
  in einen Cache; das ist **entfallen**. Gebunden und gestartet wird das
  **installierte Original** (Hash aus dem gehaltenen Handle; Linux: Start über
  `/proc/self/fd/<fd>`, Windows: nur lokale feste Laufwerke; jede Elternkomponente
  vom Laufwerk bis zum Zielordner wird nacheinander per Handle gehalten
  (Umbenennen, Löschen, Ersetzen scheitert) und am Handle auf Reparse-Punkte
  geprüft, die Datei mit Leseteilen und geprüftem Endpfad; macOS gesperrt).
- **Akzeptiertes Restfenster (persönliche Vorschau):** Auf Windows bleibt der Start
  pfadbasiert; das geprüfte Handle wird nicht atomar an den neuen Prozess
  übergeben (die Handles werden über `CreateProcess` hinaus gehalten, sodass
  Umbenennen, Löschen und Ersetzen scheitern). Inhaltsänderungen durch denselben
  Nutzer liegen außerhalb der Garantie; der Hash nach dem Lauf verwirft dann die
  Antwort.
- Ein Aufruf entspricht einem einzelnen `claude -p`-Lauf auf Anfrage des Nutzers.
  Grenzen der App: 30 Anfragen je Stunde, 200 je Tag.
- Relay iPhone → PC: Anfragen **desselben Nutzers** von dessen gekoppeltem iPhone
  an dessen eigene CLI. Erzwungen wird das durch die Geräteliste des Hubs
  desselben Kontos, einen Relay-Schlüssel je iPhone (Einrichtungscode am
  PC-Bildschirm) und das Siegel mit dem Benutzer als AAD
  (`SUBSCRIPTION_RELAY_SAME_ACCOUNT_ONLY` in `sync/booktalk_relay.rs`).
- Codex ist in der Vorschau gesperrt (Phase 2), **nicht bewertet**.

## Ergebnis (drei getrennte Aussagen)

**(1) Eigener Login in der unveränderten CLI ist ausdrücklich gestützt.**
Persönliche Nutzung durch Patrick auf eigenem Gerät, mit eigener, unveränderter
Claude-Code-Installation und eigenem Login, entspricht dem in Zitat 2 ausdrücklich
genannten Fall („end user signing in to the unmodified Claude Code binary with
their own Claude subscription“). Die Nutzung muss „ordinary, individual usage“
(Zitat 4) bleiben; dafür gelten die App-Grenzen 30/h und 200/Tag.

**(2) Die lokale Kopie ist entfallen; die App startet das installierte Original.**
Damit entfällt die Frage, ob „installed and run as published“ (Zitat 3) die
Ausführung aus einer lokalen Kopie abdeckt. Es bleibt die Frage, ob das
Starten über ein gehaltenes Datei-Handle (Linux `/proc/self/fd`) noch „run as
published“ ist; es ist dieselbe unveränderte Datei, nur der Pfad der
Ausführung ist ein anderer. Das ist nicht ausdrücklich geregelt und wird als
**Restrisiko** geführt. Grenze der Absicherung: Die Bindung an das Handle
verhindert Pfadaustausch, aber **keine Inhaltsänderung zwischen Hash und Start
durch denselben Nutzer**; diese liegt außerhalb der Garantie, der Hash nach dem
Lauf verwirft die Antwort dann.

**(3) Produktbetrieb erfordert die Commercial Terms, nicht erst die Weitergabe der
Datei.** Schon das Ausführen von Claude Code **in einem Produkt** (Zitat 3), also
eine Wolff.ai-App, die die CLI für Dritte startet, setzt die Zustimmung zu den
Commercial Terms voraus, auch wenn die Datei nie mitgeliefert wird. Jeder Nutzer
muss sich mit eigenem Abo selbst authentifizieren; Bündeln, Weiterverkaufen oder
Vermitteln von Nutzung (Zitat 1 und 3) ist ausgeschlossen. Der Relay-Fall
(„route requests … on behalf of their users“) betrifft nur Fremdnutzer: bei einer
Produktfreigabe für Dritte Relay mit Abo sperren oder mit Anthropic klären.

## Entscheidung und Verantwortung

Patrick entscheidet und trägt das Risiko als Verantwortlicher. Rahmen:

- **Standard aus.** Einschaltbar nur ausdrücklich, nach Risikohinweis im Dialog,
  und **jederzeit abschaltbar** (Ausschalten löscht die Bindung).
- **Gesperrt bis zur Klärung:** Mehrbenutzerbetrieb, fremde Konten, Weitergabe der
  App mit dieser Funktion, Produktfreigabe. **Ausdrücklich ausgenommen** ist das
  eigene iPhone desselben Kontos.
- Klärung bei Anthropic (contact sales) vor jeder Produktfreigabe.

## Wiedervorlage

- **06.02.2027** (drei Monate nach dieser Bewertung),
- außerdem **bei jeder Änderung der Bedingungen oder der Integration**,
- und **zwingend vor einer Produktfreigabe**.

## Offene Punkte

- Restrisiko Handle-Start (siehe Aussage 2), vor Produktfreigabe bei Anthropic
  klären.
- Relay mit Abo bei Fremdnutzern: sperren oder klären.
- Codex/ChatGPT: Phase 2, nicht bewertet.
- Datenschutz: Anfragen gehen über die CLI an Anthropic; der Risikohinweis im
  Einschaltdialog ist bei der Wiedervorlage zu prüfen.
