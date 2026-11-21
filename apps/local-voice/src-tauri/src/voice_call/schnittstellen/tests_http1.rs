use std::io::{self, Cursor, ErrorKind, Read};

use super::anbindung::Ziel;
use super::http1::{baue_anfragebytes, lies_antwort, Rohantwort};
use super::kodierung::Wert;
use super::objekt;

struct Zaehlleser {
    daten: Cursor<Vec<u8>>,
    gelesen: usize,
    unerwartetes_ende: bool,
}

impl Zaehlleser {
    fn neu(daten: impl AsRef<[u8]>) -> Self {
        Self {
            daten: Cursor::new(daten.as_ref().to_vec()),
            gelesen: 0,
            unerwartetes_ende: false,
        }
    }
}

impl Read for Zaehlleser {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let n = self.daten.read(bytes)?;
        self.gelesen += n;
        if n == 0 && self.unerwartetes_ende {
            Err(ErrorKind::UnexpectedEof.into())
        } else {
            Ok(n)
        }
    }
}

struct Langsam {
    daten: Cursor<Vec<u8>>,
    versuche: usize,
    fehler: ErrorKind,
}

impl Read for Langsam {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.versuche += 1;
        if self.versuche <= 3 {
            Err(self.fehler.into())
        } else {
            self.daten.read(bytes)
        }
    }
}

fn antwort(daten: impl AsRef<[u8]>, max: usize) -> Result<Rohantwort, &'static str> {
    lies_antwort(&mut Cursor::new(daten.as_ref().to_vec()), max, &mut || {
        Ok(())
    })
}

fn erwartet(status: u16, koerper: &[u8]) -> Result<Rohantwort, &'static str> {
    Ok(Rohantwort {
        status,
        koerper: koerper.to_vec(),
    })
}

fn anfrage(methode: &str, adresse: &str, zeilen: &[(&str, &str)], koerper: Wert) -> Wert {
    objekt([
        ("methode", methode.into()),
        ("adresse", adresse.into()),
        (
            "kopfzeilen",
            Wert::Liste(
                zeilen
                    .iter()
                    .map(|(name, wert)| Wert::Liste(vec![(*name).into(), (*wert).into()]))
                    .collect(),
            ),
        ),
        ("koerper", koerper),
    ])
}

#[test]
fn anfrage_bytes_exakt() {
    // Schuetzt Reihenfolge, CRLF, Originalhost, Port und UTF-8-Bytezahl des Koerpers.
    let mut ziel = Ziel {
        host: "ursprung.test".into(),
        port: 443,
        tls: true,
    };
    let get = anfrage("GET", "https://adresse.test", &[], Wert::Null);
    assert_eq!(
        baue_anfragebytes(&ziel, &get).unwrap(),
        b"GET / HTTP/1.1\r\nHost: ursprung.test\r\nConnection: close\r\n\r\n"
    );
    ziel.port = 8443;
    let post = anfrage(
        "POST",
        "https://adresse.test:8443/pfad?q=1",
        &[("Content-Type", "application/json"), ("X-Test", "ja")],
        "{\"a\":\"\u{e4}\"}".into(),
    );
    let bytes = baue_anfragebytes(&ziel, &post).unwrap();
    assert_eq!(
        bytes,
        concat!(
            "POST /pfad?q=1 HTTP/1.1\r\nHost: ursprung.test:8443\r\n",
            "Connection: close\r\nContent-Type: application/json\r\nX-Test: ja\r\n",
            "Content-Length: 10\r\n\r\n{\"a\":\"\u{e4}\"}"
        )
        .as_bytes()
    );
    for verboten in ["Accept-Encoding", "Upgrade", "Transfer-Encoding", "Expect"] {
        assert!(!String::from_utf8_lossy(&bytes).contains(verboten));
    }
    ziel.port = 80;
    ziel.tls = false;
    assert_eq!(
        baue_anfragebytes(&ziel, &anfrage("GET", "http://a/", &[], Wert::Null)).unwrap(),
        b"GET / HTTP/1.1\r\nHost: ursprung.test\r\nConnection: close\r\n\r\n"
    );
    // Schuetzt Methoden- und Zielvalidierung gegen Injektion und andere Protokolle.
    for adresse in [
        "https://a/a b",
        "https://a/a\rb",
        "https://a/a\nb",
        "https://a/a\u{1f}",
        "https://a/a\u{7f}",
        "https://a/a#b",
        "https://u@a/",
        "ftp://a/",
    ] {
        assert_eq!(
            baue_anfragebytes(&ziel, &anfrage("GET", adresse, &[], Wert::Null)),
            Err("netzfehler")
        );
    }
    assert_eq!(
        baue_anfragebytes(&ziel, &anfrage("CONNECT", "https://a/", &[], Wert::Null)),
        Err("netzfehler")
    );
    for methode in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
        assert!(baue_anfragebytes(&ziel, &anfrage(methode, "http://a/", &[], Wert::Null)).is_ok());
    }
    // Schuetzt alle reservierten Kopfnamen, auch unabhaengig von der Schreibweise.
    for name in [
        "host",
        "connection",
        "content-length",
        "transfer-encoding",
        "upgrade",
        "te",
        "expect",
        "accept-encoding",
        "proxy-connection",
        "keep-alive",
        "trailer",
        "HOST",
        "Content-Length",
        "",
        "X_Test",
        "X:Test",
    ] {
        let a = anfrage("GET", "http://a/", &[(name, "x")], Wert::Null);
        assert_eq!(baue_anfragebytes(&ziel, &a), Err("netzfehler"));
    }
    // Schuetzt Kopfwerte vor CR/LF, Steuerzeichen und Nicht-ASCII.
    for wert in ["a\rb", "a\nb", "a\u{1f}", "a\u{7f}", "\u{e4}"] {
        let a = anfrage("GET", "http://a/", &[("X-Test", wert)], Wert::Null);
        assert_eq!(baue_anfragebytes(&ziel, &a), Err("netzfehler"));
    }
}

#[test]
fn antwort_content_length() {
    // Schuetzt exakte Laenge, identische doppelte Laenge und ignorierten Ueberhang.
    let kopf = "HTTP/1.0 200 OK\r\ncOnTeNt-LeNgTh: 3\r\nContent-Length: 3\r\n\r\n";
    let mut quelle = Zaehlleser::neu(format!("{kopf}abcREST"));
    assert_eq!(
        lies_antwort(&mut quelle, 3, &mut || Ok(())),
        erwartet(200, b"abc")
    );
    assert_eq!(quelle.gelesen, kopf.len() + 3);
    assert_eq!(
        antwort(
            "HTTP/1.1 200 OK\r\nContent-Encoding: IDENTITY\r\nContent-Length: 0\r\n\r\n",
            0
        ),
        erwartet(200, b"")
    );
}

#[test]
fn antwort_chunked() {
    // Schuetzt Hexgroessen, Erweiterungen, mehrere Chunks, CRLF und Trailerabschluss.
    assert_eq!(
        antwort(
            concat!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: ChUnKeD\r\n\r\n",
                "A; test=ja\r\n0123456789\r\n2\r\nab\r\n0\r\nX-Test: ja\r\n\r\n"
            ),
            12
        ),
        erwartet(200, b"0123456789ab")
    );
}

#[test]
fn antwort_bis_verbindungsende() {
    // Schuetzt beide erlaubten Verbindungsenden ohne Laengenangabe.
    for unerwartetes_ende in [false, true] {
        let mut quelle = Zaehlleser::neu("HTTP/1.1 200 OK\r\n\r\nabc");
        quelle.unerwartetes_ende = unerwartetes_ende;
        assert_eq!(
            lies_antwort(&mut quelle, 3, &mut || Ok(())),
            erwartet(200, b"abc")
        );
    }
}

#[test]
fn antwort_ohne_koerper_bei_204_304_und_1xx() {
    // Schuetzt koerperlose Statuscodes und verhindert Lesen nach dem Kopf, auch bei 101.
    for status in [100, 101, 199, 204, 304] {
        let kopf = format!("HTTP/1.1 {status} Status\r\nContent-Length: 999\r\n\r\n");
        let mut quelle = Zaehlleser::neu(format!("{kopf}NICHT LESEN"));
        assert_eq!(
            lies_antwort(&mut quelle, 0, &mut || Ok(())),
            erwartet(status, b"")
        );
        assert_eq!(quelle.gelesen, kopf.len());
    }
}

#[test]
fn zu_gross() {
    // Schuetzt Abbruch vor Daten bei bekannter Laenge und bei jeder Chunkgroesse.
    for kopf in [
        "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nab\r\n2\r\n",
    ] {
        let mut quelle = Zaehlleser::neu(format!("{kopf}DATEN"));
        assert_eq!(
            lies_antwort(&mut quelle, 3, &mut || Ok(())),
            Err("zu_gross")
        );
        assert_eq!(quelle.gelesen, kopf.len());
    }
    // Schuetzt die einzelne Probe jenseits der Grenze bei unbekannter Laenge.
    let kopf = "HTTP/1.1 200 OK\r\n\r\n";
    let mut quelle = Zaehlleser::neu(format!("{kopf}abcde"));
    assert_eq!(
        lies_antwort(&mut quelle, 3, &mut || Ok(())),
        Err("zu_gross")
    );
    assert_eq!(quelle.gelesen, kopf.len() + 4);
    // Schuetzt die feste Transportgrenze auch bei einem zu grossen Aufruferlimit.
    let kopf = "HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\n\r\n";
    let mut quelle = Zaehlleser::neu(format!("{kopf}DATEN"));
    assert_eq!(
        lies_antwort(&mut quelle, usize::MAX, &mut || Ok(())),
        Err("zu_gross")
    );
    assert_eq!(quelle.gelesen, kopf.len());
    // Schuetzt die inklusive Grenze fuer alle drei Rahmungsarten, auch bei Null.
    for (daten, max, koerper) in [
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc",
            3,
            &b"abc"[..],
        ),
        ("HTTP/1.1 200 OK\r\n\r\nabc", 3, &b"abc"[..]),
        (
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
            3,
            &b"abc"[..],
        ),
        ("HTTP/1.1 200 OK\r\n\r\n", 0, &b""[..]),
    ] {
        assert_eq!(antwort(daten, max), erwartet(200, koerper));
    }
}

#[test]
fn kaputte_rahmung() {
    // Schuetzt Statussyntax, CRLF, Kopfsyntax, Laengen und vorzeitigen Abbruch.
    for daten in [
        "200 OK\r\n\r\n",
        "HTTP/1.1 99 Status\r\n\r\n",
        "HTTP/1.1 600 Status\r\n\r\n",
        "HTTP/1.1 abc Status\r\n\r\n",
        "HTTP/1.1 200 OK\n\n",
        "HTTP/1.1 200 OK\r\nX: a\n\r\n",
        "HTTP/1.1 200 OK\r\nX ohne Doppelpunkt\r\n\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nab",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nab",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\naX\r\n0\r\n\r\n",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\nX: ja\r\n",
    ] {
        assert_eq!(antwort(daten, 100), Err("netzfehler"));
    }
    // Schuetzt Dezimal- und Hexsyntax einschliesslich Ueberlauf.
    for laenge in ["+5", "5x", "", "999999999999999999999999999999999999"] {
        let daten = format!("HTTP/1.1 200 OK\r\nContent-Length: {laenge}\r\n\r\n");
        assert_eq!(antwort(daten, 100), Err("netzfehler"));
    }
    for groesse in [
        "-1",
        "0x5",
        "zz",
        " 5",
        "5 ",
        "",
        "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
    ] {
        let daten = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{groesse}\r\n");
        assert_eq!(antwort(daten, 100), Err("netzfehler"));
    }
    // Schuetzt getrennte Grenzen fuer Kopf und Trailer.
    let kopf = format!("HTTP/1.1 200 OK\r\nX: {}\r\n\r\n", "a".repeat(16 * 1024));
    assert_eq!(antwort(kopf, 100), Err("netzfehler"));
    let trailer = format!(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\nX: {}\r\n\r\n",
        "a".repeat(16 * 1024)
    );
    assert_eq!(antwort(trailer, 100), Err("netzfehler"));
    // Schuetzt Ablehnung ungeforderter Inhaltskodierung und anderer Transferkodierungen.
    for kopf in ["Content-Encoding: gzip", "Transfer-Encoding: gzip, chunked"] {
        assert_eq!(
            antwort(format!("HTTP/1.1 200 OK\r\n{kopf}\r\n\r\n"), 100),
            Err("antwort_ungueltig")
        );
    }
}

#[test]
fn alt_svc_location_upgrade_werden_ignoriert() {
    // Schuetzt die reine Rueckgabe von Status und Koerper trotz Wechselangeboten.
    assert_eq!(
        antwort(
            concat!(
                "HTTP/1.1 302 Found\r\nLocation: https://anders.test/\r\n",
                "Alt-Svc: h3=\":8443\"\r\nUpgrade: h2c\r\nConnection: Upgrade\r\n",
                "Content-Length: 3\r\n\r\nabc"
            ),
            3
        ),
        erwartet(302, b"abc")
    );
    // Schuetzt das Verwerfen unbekannter Kopfwerte ohne UTF-8-Auswertung.
    assert_eq!(
        antwort(
            b"HTTP/1.1 200 OK\r\nX_Unbekannt: \xff\r\nContent-Length: 0\r\n\r\n",
            0
        ),
        erwartet(200, b"")
    );
}

#[test]
fn warten_rueckruf() {
    // Schuetzt erneutes Lesen nach Bereitschaft und direktes Weiterreichen von Abbruchfehlern.
    for fehler in [ErrorKind::WouldBlock, ErrorKind::TimedOut] {
        let mut quelle = Langsam {
            daten: Cursor::new(b"HTTP/1.1 200 OK\r\n\r\nabc".to_vec()),
            versuche: 0,
            fehler,
        };
        let mut aufrufe = 0;
        let ergebnis = lies_antwort(&mut quelle, 3, &mut || {
            aufrufe += 1;
            Ok(())
        });
        assert_eq!(ergebnis, erwartet(200, b"abc"));
        assert_eq!(aufrufe, 3);
        for code in ["zeitlimit", "abgebrochen"] {
            quelle.versuche = 0;
            assert_eq!(lies_antwort(&mut quelle, 3, &mut || Err(code)), Err(code));
            assert_eq!(quelle.versuche, 1);
        }
    }
    // Schuetzt Wiederholung bei Interrupted ohne Aufruf des Wartepfads.
    let mut quelle = Langsam {
        daten: Cursor::new(b"HTTP/1.1 200 OK\r\n\r\n".to_vec()),
        versuche: 0,
        fehler: ErrorKind::Interrupted,
    };
    assert_eq!(
        lies_antwort(&mut quelle, 0, &mut || Err("abgebrochen")),
        erwartet(200, b"")
    );
}
