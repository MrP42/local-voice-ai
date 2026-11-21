# Prüfstand für den Sprachdienst

Führt die Tests des Sprachdienstes aus, ohne die App zu bauen. Gedacht für Linux und macOS
(erprobt unter Linux), also auch für Umgebungen, in denen die App nicht baut, weil GTK oder WebKit
fehlen (Cloud-Sitzungen).

```bash
cargo test --locked --manifest-path apps/local-voice/crates/sprachdienst-pruefstand/Cargo.toml
```

## Was hier geprüft wird

Dieses Crate enthält keinen Dienstcode. `src/sprachdienst/mod.rs` bindet die Originaldateien aus
`src-tauri/src/sprachdienst` per `#[path]` ein, unter demselben Modulpfad wie in der App
(`crate::sprachdienst::…`, `crate::testlog`). Die Tests sind die aus `sprachdienst/tests.rs`,
unverändert. Der Vertrag kommt aus derselben vendorten Crate (`src-tauri/vendor/wai-voice-dienst`).

## Abweichungen vom echten Crate

Der Prüfstand ersetzt das Rust-Tor im App-Crate nicht. Er weicht in diesen Punkten ab:

| Punkt                                                                | App-Crate              | Prüfstand                                   |
| -------------------------------------------------------------------- | ---------------------- | ------------------------------------------- |
| `sprachdienst/app.rs` (Engines, Ereignisse, Fenster)                 | gebaut                 | nicht eingebunden                           |
| `sprachdienst/pipe.rs` (Windows-Pipe, DACL, Prüfung der Gegenstelle) | gebaut unter Windows   | nicht eingebunden                           |
| Betriebssystem                                                       | Windows, macOS, Linux  | nur Unix (unter Windows ist das Crate leer) |
| Abhängigkeiten                                                       | `src-tauri/Cargo.toml` | Teilmenge davon, zeilengleich (Wächter)     |
| Features und `cfg` der App                                           | wirksam                | nicht vorhanden                             |
| Übrige Module der App                                                | im selben Crate        | fehlen; ein Zugriff darauf baut hier nicht  |

Folge: Grün im Prüfstand heißt, dass Protokoll, Zugang, Kopplung, Fristen und der Unix-Endpunkt
ihre Tests bestehen. Es sagt nichts über die Anbindung an die App und nichts über Windows.

## Wächter gegen Drift

Die Tests in `src/waechter.rs` prüfen nicht den Dienst, sondern den Prüfstand. Jeder Wächter ist
eine reine Funktion, die Abweichungen als Text liefert, dazu ein Test gegen die echten Dateien und
eine Gegenprobe mit erfundenen Eingaben (`gegenprobe_…`), die bei jedem Lauf belegt, dass die
Abweichung auch gemeldet wird.

- `module_folgen_dem_dienst`: Der Spiegel `src/sprachdienst/mod.rs` wird aus
  `sprachdienst/mod.rs` der App **abgeleitet** und muss dem Ergebnis genau gleichen: je Modul
  dieselbe Sichtbarkeit, dieselbe Bedingung (`cfg(unix)` entfällt, weil das ganze Crate darunter
  steht) und der `#[path]` zur gleichnamigen Originaldatei. Rot wird der Test bei einer Umlenkung
  auf eine andere Datei, einem anderen `cfg`, einem fehlenden oder zusätzlichen Modul und bei jeder
  Zeile, die keine Modulzeile ist, im Spiegel wie im Original. Trägt ein Modul im Original ein
  Attribut, das der Prüfstand nicht abbildet (etwa `cfg(feature = …)`), ist das ebenfalls rot.
- `wurzel_ist_unveraendert`: `src/lib.rs` enthält nur `#![cfg(unix)]` und die drei Modulzeilen.
- `einbindung_in_der_app_ist_unveraendert`: Die App bindet `sprachdienst` weiter ohne Bedingung
  und `testlog` nur für Tests ein.
- `abhaengigkeiten_folgen_der_app`: Beide `Cargo.toml` werden von Cargo selbst gelesen
  (`cargo metadata --no-deps`). **Jede** Abhängigkeit des Prüfstands, auch eine neu eingetragene,
  muss in der App mit derselben Art (normal, `dev`, `build`) genau einmal stehen und in
  Anforderung, Quelle (Register oder Pfad), Features, Standard-Features, Umbenennung, `optional`
  und Zielbedingung übereinstimmen. Eigene Features hat der Prüfstand nicht.
- `sperrdatei_folgt_der_app`: Jedes Paket in `Cargo.lock` hier steht mit derselben Version,
  **Quelle und Prüfsumme** in `src-tauri/Cargo.lock`. Seine Kanten werden in jeder Datei zur vollen
  Identität des Ziels aufgelöst (Name, Version, Quelle) und müssen auch in der App so bestehen:
  Zeigt ein Paket hier auf `b 1` und in der App auf `b 2`, ist das rot, auch wenn die App beide
  Versionen sperrt.

Beabsichtigte Unterschiede, die die Wächter nicht melden: Der Prüfstand lässt `app` und `pipe`
aus und nennt nur einen Teil der Abhängigkeiten der App. Weitere sind nicht vorgesehen; wer einen
braucht, trägt ihn im Wächter ein und oben in die Tabelle.

### Was die Wächter nicht leisten

- **Änderungen an der Modulwurzel des Dienstes brauchen weiterhin ein Review.** Der Wächter
  erkennt, dass sich `sprachdienst/mod.rs` in einer Form geändert hat, die der Spiegel nicht
  abbildet. Ob der Prüfstand danach noch dasselbe prüft wie die App, entscheidet ein Mensch.
- Der Spiegel **ersetzt** die Modulwurzel: Was dort außer Modulzeilen stünde, gäbe es hier nicht.
  Heute steht dort nichts anderes, und der Wächter meldet, sobald sich das ändert.
- Die Wächter vergleichen Erklärungen. Welche Features Cargo in der App am Ende einschaltet (dort
  schalten andere Pakete weitere Features derselben Abhängigkeiten ein), bildet der Prüfstand
  nicht nach. Deshalb bleibt das Rust-Tor im App-Crate maßgeblich.
- Die Modulzeilen werden zeilenweise gelesen, nicht mit einem Rust-Parser. Der Leser versteht nur
  Kommentarzeilen, einzeilige Attribute und `mod x;` / `pub mod x;` und meldet alles andere.

## Tor für Änderungen am Prüfstand

Wer den Prüfstand selbst ändert, belegt zwei Läufe:

```bash
# unter Linux oder macOS
cargo test --locked --manifest-path apps/local-voice/crates/sprachdienst-pruefstand/Cargo.toml
# dort, wo die App baut (Windows)
cargo check --locked --tests --manifest-path apps/local-voice/src-tauri/Cargo.toml
```

Der zweite Lauf prüft auch den Testcode der App (`cfg(test)`), den ein gewöhnliches `cargo check`
auslässt.

Der Beleg des ersten Laufs nennt die Wächter **einzeln** (`cargo test --locked … waechter` listet
sie): `module_folgen_dem_dienst`, `wurzel_ist_unveraendert`, `einbindung_in_der_app_ist_unveraendert`,
`abhaengigkeiten_folgen_der_app`, `sperrdatei_folgt_der_app` und die drei `gegenprobe_…`. Eine
bloße Gesamtzahl genügt nicht: Die Wächter stehen selbst unter `#[cfg(test)] mod waechter;` in
`src/lib.rs`. Wer diese Bedingung abschaltet, schaltet auch den Test ab, der die Wurzel prüft; das
fällt nur auf, wenn die Namen im Beleg fehlen.

## Sperrdatei neu ableiten

Ist `sperrdatei_folgt_der_app` rot, weil sich die Sperrdatei der App geändert hat, die Sperrdatei
hier neu ableiten (Cargo streicht alles, was der Prüfstand nicht braucht, und behält Versionen,
Quellen und Prüfsummen):

```bash
cd apps/local-voice/crates/sprachdienst-pruefstand
cp ../../src-tauri/Cargo.lock Cargo.lock
cargo metadata --format-version 1 > /dev/null
```

## Wenn der Bau hier scheitert, in der App aber nicht

Dann greift ein Dienstmodul auf etwas außerhalb von `sprachdienst` zu (`crate::…`). Das ist kein
Fehler des Dienstes. Entweder den Zugriff in `app.rs` halten, wo die Anbindung an die App liegt,
oder den Prüfstand erweitern und die Abweichung oben eintragen.
