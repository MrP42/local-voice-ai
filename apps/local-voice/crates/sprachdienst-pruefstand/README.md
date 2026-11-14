# Prüfstand für den Sprachdienst

Führt die Tests des Sprachdienstes aus, ohne die App zu bauen. Gedacht für Linux und macOS
(erprobt unter Linux), also auch für Umgebungen, in denen die App nicht baut, weil GTK oder WebKit
fehlen (Cloud-Sitzungen).

```bash
cargo test --manifest-path apps/local-voice/crates/sprachdienst-pruefstand/Cargo.toml
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
| Abhängigkeiten                                                       | `src-tauri/Cargo.toml` | eigene Liste, nur was der Dienst braucht    |
| Features und `cfg` der App                                           | wirksam                | nicht vorhanden                             |
| Übrige Module der App                                                | im selben Crate        | fehlen; ein Zugriff darauf baut hier nicht  |

Folge: Grün im Prüfstand heißt, dass Protokoll, Zugang, Kopplung, Fristen und der Unix-Endpunkt
ihre Tests bestehen. Es sagt nichts über die Anbindung an die App und nichts über Windows.

## Wächter gegen Drift

Drei Tests in `src/lib.rs` (`waechter::…`) prüfen nicht den Dienst, sondern den Prüfstand:

- `module_folgen_dem_dienst`: `sprachdienst/mod.rs` der App und der Spiegel hier nennen dieselben
  Module, ausgenommen `app` und `pipe`. Ein neues Modul im Dienst macht den Test rot, bis es hier
  eingebunden oder bewusst ausgenommen ist.
- `abhaengigkeiten_folgen_der_app`: Jede Abhängigkeit hier fordert eine Version an, die auch
  `src-tauri/Cargo.toml` anfordert.
- `sperrdatei_folgt_der_app`: Jede Paketversion in `Cargo.lock` hier steht auch in
  `src-tauri/Cargo.lock`.

Ist der dritte Test rot, weil sich die Sperrdatei der App geändert hat, die Sperrdatei hier neu
ableiten (Cargo streicht alles, was der Prüfstand nicht braucht, und behält die Versionen):

```bash
cd apps/local-voice/crates/sprachdienst-pruefstand
cp ../../src-tauri/Cargo.lock Cargo.lock
cargo metadata --format-version 1 > /dev/null
```

## Wenn der Bau hier scheitert, in der App aber nicht

Dann greift ein Dienstmodul auf etwas außerhalb von `sprachdienst` zu (`crate::…`). Das ist kein
Fehler des Dienstes. Entweder den Zugriff in `app.rs` halten, wo die Anbindung an die App liegt,
oder den Prüfstand erweitern und die Abweichung oben eintragen.
