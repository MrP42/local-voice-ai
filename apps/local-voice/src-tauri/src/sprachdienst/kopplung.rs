//! Offene Kopplungsanfragen (ADR-0021, Protokoll "Kopplung").
//!
//! Eine App schickt `koppeln {client, anzeige, fingerabdruck, code}`. Local Voice AI zeigt Name und
//! den sechsstelligen Code; der Nutzer bestätigt nur, wenn der Code mit dem in der App
//! übereinstimmt. Das hier ist der Zustand dazwischen:
//!
//! - **Bestätigung nur in der Oberfläche**: [`Kopplungen::entscheiden`] wird ausschließlich von
//!   einem Tauri-Kommando gerufen. Über die Leitung gibt es keinen Weg dorthin.
//! - **Ablauf** nach `KOPPELN_BESTAETIGUNG_S` (120 s): die wartende Verbindung bekommt
//!   `zeit_ueberschritten`.
//! - **Ratenbremse**: je Client-Kennung höchstens `KOPPELN_JE_MINUTE` (3) Versuche im
//!   Gleitfenster einer Minute und höchstens eine offene Anfrage zugleich, sonst `beschaeftigt`.
//!   Da die Client-Kennung vom Anrufer stammt, gilt zusätzlich eine Grenze über alle Clients
//!   (höchstens 4 offene Dialoge, 12 Versuche je Minute): ein fremder Prozess desselben Benutzers
//!   kann den Nutzer nicht mit Dialogen fluten, indem er Kennungen wechselt.
//! - Ein schon gekoppelter Fingerabdruck bekommt `bereits_gekoppelt` (kein zweiter Zugang).
//!
//! Die Uhr kommt als Parameter (Millisekunden), damit Tests ohne Warten auskommen.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use tokio::sync::oneshot;
use wai_voice_dienst::grenzen;
use wai_voice_dienst::msg::code;

/// Fenster der Ratenbremse.
pub const FENSTER_MS: i64 = 60_000;
/// Höchstzahl gleichzeitig offener Dialoge über alle Clients.
pub const OFFEN_MAX: usize = 4;
/// Höchstzahl Versuche je Minute über alle Clients.
pub const GESAMT_JE_MINUTE: usize = 12;
/// Höchstzahl verfolgter Client-Kennungen (älteste Fenster werden vergessen).
const KENNUNGEN_MAX: usize = 256;

/// Was die Oberfläche von einer offenen Anfrage sieht. Kein Fingerabdruck.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OffeneKopplung {
    pub id: String,
    pub client: String,
    pub anzeige: String,
    pub code: String,
    /// Bis wann bestätigt werden kann (ms, UTC).
    pub bis_ms: i64,
}

struct Offen {
    ansicht: OffeneKopplung,
    antwort: Option<oneshot::Sender<bool>>,
}

#[derive(Default)]
struct Innen {
    offen: Vec<Offen>,
    je_client: HashMap<String, VecDeque<i64>>,
    gesamt: VecDeque<i64>,
    zaehler: u64,
}

#[derive(Default)]
pub struct Kopplungen {
    innen: Mutex<Innen>,
}

/// Wartet die Verbindung auf die Entscheidung des Nutzers.
pub struct Warten {
    pub id: String,
    pub antwort: oneshot::Receiver<bool>,
}

fn fenster_leeren(q: &mut VecDeque<i64>, jetzt_ms: i64) {
    while q.front().is_some_and(|t| jetzt_ms - *t >= FENSTER_MS) {
        q.pop_front();
    }
}

impl Kopplungen {
    pub fn new() -> Self {
        Self::default()
    }

    fn sperre(&self) -> std::sync::MutexGuard<'_, Innen> {
        self.innen.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Nimmt eine Anfrage an oder weist sie mit einem Fehlercode des Vertrags ab.
    ///
    /// Jeder Versuch zählt für die Ratenbremse, auch ein abgewiesener. `bereits`: Der
    /// Fingerabdruck gehört schon zu einem Zugang. `bestaetigung_ms`: Frist der Anfrage.
    pub fn anfragen(
        &self,
        client: &str,
        anzeige: &str,
        code_text: &str,
        bereits: bool,
        jetzt_ms: i64,
        bestaetigung_ms: i64,
    ) -> Result<Warten, &'static str> {
        let mut i = self.sperre();
        // Abgelaufene Anfragen räumen, damit ein vergessener Dialog den Platz nicht blockiert.
        i.offen.retain(|o| o.ansicht.bis_ms > jetzt_ms);

        if !i.je_client.contains_key(client) && i.je_client.len() >= KENNUNGEN_MAX {
            i.je_client.retain(|_, q| {
                fenster_leeren(q, jetzt_ms);
                !q.is_empty()
            });
            if i.je_client.len() >= KENNUNGEN_MAX {
                return Err(code::BESCHAEFTIGT);
            }
        }
        let q = i.je_client.entry(client.to_string()).or_default();
        fenster_leeren(q, jetzt_ms);
        let zu_oft = q.len() >= grenzen::KOPPELN_JE_MINUTE as usize;
        q.push_back(jetzt_ms);
        fenster_leeren(&mut i.gesamt, jetzt_ms);
        let gesamt_zu_oft = i.gesamt.len() >= GESAMT_JE_MINUTE;
        i.gesamt.push_back(jetzt_ms);
        if zu_oft || gesamt_zu_oft {
            return Err(code::BESCHAEFTIGT);
        }
        if bereits {
            return Err(code::BEREITS_GEKOPPELT);
        }
        if i.offen.iter().any(|o| o.ansicht.client == client) || i.offen.len() >= OFFEN_MAX {
            return Err(code::BESCHAEFTIGT);
        }

        i.zaehler += 1;
        let id = format!("k{}", i.zaehler);
        let (tx, rx) = oneshot::channel();
        i.offen.push(Offen {
            ansicht: OffeneKopplung {
                id: id.clone(),
                client: client.to_string(),
                anzeige: anzeige.to_string(),
                code: code_text.to_string(),
                bis_ms: jetzt_ms + bestaetigung_ms,
            },
            antwort: Some(tx),
        });
        Ok(Warten { id, antwort: rx })
    }

    /// Entscheidung des Nutzers (nur aus der Oberfläche). `false`: die Anfrage gibt es nicht
    /// (mehr), etwa weil sie abgelaufen ist oder die App aufgelegt hat.
    pub fn entscheiden(&self, id: &str, bestaetigt: bool, jetzt_ms: i64) -> bool {
        let mut i = self.sperre();
        let Some(pos) = i.offen.iter().position(|o| o.ansicht.id == id) else {
            return false;
        };
        let mut o = i.offen.remove(pos);
        if o.ansicht.bis_ms <= jetzt_ms {
            return false;
        }
        o.antwort
            .take()
            .is_some_and(|tx| tx.send(bestaetigt).is_ok())
    }

    /// Anfrage beenden (Entscheidung, Ablauf oder aufgelegt). Idempotent.
    pub fn aufgeben(&self, id: &str) {
        self.sperre().offen.retain(|o| o.ansicht.id != id);
    }

    /// Offene, noch nicht abgelaufene Anfragen für die Oberfläche.
    pub fn offen(&self, jetzt_ms: i64) -> Vec<OffeneKopplung> {
        let mut i = self.sperre();
        i.offen.retain(|o| o.ansicht.bis_ms > jetzt_ms);
        i.offen.iter().map(|o| o.ansicht.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRIST: i64 = grenzen::KOPPELN_BESTAETIGUNG_S as i64 * 1000;

    fn frage(k: &Kopplungen, client: &str, jetzt: i64) -> Result<Warten, &'static str> {
        k.anfragen(client, "Corporate LLM", "123456", false, jetzt, FRIST)
    }

    #[test]
    fn bestaetigung_erreicht_die_wartende_verbindung() {
        let k = Kopplungen::new();
        let mut w = frage(&k, "corporate-llm", 0).unwrap();
        let liste = k.offen(1);
        assert_eq!(liste.len(), 1);
        assert_eq!(liste[0].code, "123456");
        assert_eq!(liste[0].bis_ms, FRIST);
        assert!(k.entscheiden(&w.id, true, 10));
        assert_eq!(w.antwort.try_recv(), Ok(true));
        assert!(k.offen(11).is_empty());
        assert!(!k.entscheiden(&w.id, true, 12), "nur einmal");
    }

    #[test]
    fn ablehnen() {
        let k = Kopplungen::new();
        let mut w = frage(&k, "a", 0).unwrap();
        assert!(k.entscheiden(&w.id, false, 5));
        assert_eq!(w.antwort.try_recv(), Ok(false));
    }

    #[test]
    fn nach_120_sekunden_ist_nichts_mehr_zu_bestaetigen() {
        let k = Kopplungen::new();
        let w = frage(&k, "a", 0).unwrap();
        assert_eq!(k.offen(FRIST - 1).len(), 1);
        assert!(k.offen(FRIST).is_empty(), "abgelaufen");
        assert!(!k.entscheiden(&w.id, true, FRIST), "zu spät");
    }

    #[test]
    fn eine_offene_je_client_sonst_beschaeftigt() {
        let k = Kopplungen::new();
        let w = frage(&k, "a", 0).unwrap();
        assert_eq!(frage(&k, "a", 1).err(), Some(code::BESCHAEFTIGT));
        assert!(
            frage(&k, "b", 2).is_ok(),
            "ein anderer Client ist nicht betroffen"
        );
        k.aufgeben(&w.id);
        assert!(frage(&k, "a", 3).is_ok(), "nach dem Beenden wieder frei");
    }

    #[test]
    fn hoechstens_drei_versuche_je_minute() {
        let k = Kopplungen::new();
        for t in 0..3 {
            let w = frage(&k, "a", t * 1000).unwrap();
            k.aufgeben(&w.id);
        }
        assert_eq!(
            frage(&k, "a", 3_000).err(),
            Some(code::BESCHAEFTIGT),
            "vierter Versuch"
        );
        // Auch abgewiesene Versuche zählen, das Fenster rückt also nicht weiter.
        assert_eq!(frage(&k, "a", 59_000).err(), Some(code::BESCHAEFTIGT));
        // Bei 61 s sind erst zwei Versuche aus dem Fenster; drei stehen noch darin.
        assert_eq!(frage(&k, "a", 61_000).err(), Some(code::BESCHAEFTIGT));
        // Erst wenn auch die abgewiesenen Versuche alt genug sind, geht es wieder.
        assert!(frage(&k, "a", 122_000).is_ok());
        assert!(frage(&k, "b", 3_000).is_ok(), "je Client getrennt");
    }

    #[test]
    fn bereits_gekoppelt_hat_eigenen_code_und_zaehlt_als_versuch() {
        let k = Kopplungen::new();
        let r = k.anfragen("a", "A", "123456", true, 0, FRIST);
        assert_eq!(r.err(), Some(code::BEREITS_GEKOPPELT));
        assert!(k.offen(1).is_empty(), "kein Dialog");
    }

    #[test]
    fn dialogflut_ueber_wechselnde_kennungen_wird_gedeckelt() {
        let k = Kopplungen::new();
        let mut gehalten = Vec::new();
        for n in 0..OFFEN_MAX {
            gehalten.push(frage(&k, &format!("c{n}"), n as i64).unwrap());
        }
        assert_eq!(frage(&k, "weiterer", 10).err(), Some(code::BESCHAEFTIGT));
        assert_eq!(k.offen(11).len(), OFFEN_MAX);
        // Gesamtgrenze je Minute, auch ohne offene Dialoge.
        let k = Kopplungen::new();
        let mut abgewiesen = 0;
        for n in 0..GESAMT_JE_MINUTE + 5 {
            match frage(&k, &format!("c{n}"), n as i64) {
                Ok(w) => k.aufgeben(&w.id),
                Err(_) => abgewiesen += 1,
            }
        }
        assert_eq!(abgewiesen, 5);
    }

    #[test]
    fn aufgelegte_verbindung_haengt_nicht_im_dialog() {
        let k = Kopplungen::new();
        let w = frage(&k, "a", 0).unwrap();
        drop(w.antwort);
        assert!(!k.entscheiden(&w.id, true, 1), "Empfänger ist weg");
    }
}
