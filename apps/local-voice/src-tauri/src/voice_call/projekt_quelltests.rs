//! lva#365 P4c-2: Quelltests zum Projektzweig. Sie halten fest, was sich nur aus dem Aufbau des Codes
//! ergibt: Der Zweig steht vor dem Notizbuchschutz, ein App-Kontext wird im Projekt nicht gelesen, Werkzeuge
//! sind aus, und der Versand folgt dem Tor. Die Texte kommen aus den Quelldateien selbst.

const MOD: &str = include_str!("mod.rs");
const TURN: &str = include_str!("turn.rs");
const PROJEKT: &str = include_str!("projekt.rs");
const TOOLS: &str = include_str!("../commands/voice_tools.rs");
const CONVERSATIONS: &str = include_str!("../conversations/mod.rs");
const ZUSTAND: &str = include_str!("../projekte/zustand.rs");

/// Der Quelltext ohne den Testteil: ein eingebettetes `#[cfg(test)] mod tests { … }` fällt weg. Andere
/// `cfg(test)`-Einträge (ein Modul in einer eigenen Datei, Hilfen) stehen oft vor dem Code und zählen nicht.
/// Zeilenenden werden vereinheitlicht: ein Windows-Checkout liefert `\r\n`.
fn ohne_tests(s: &str) -> String {
    let s = s.replace("\r\n", "\n");
    s.split("\n#[cfg(test)]\nmod tests {")
        .next()
        .unwrap_or(&s)
        .to_string()
}

/// Der Code ohne Kommentarzeilen (`//`, `///`, `//!`): Erwähnungen im Kommentar zählen nicht.
fn code_ohne_kommentare(s: &str) -> String {
    s.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn stelle(s: &str, nadel: &str) -> usize {
    ohne_tests(s)
        .find(nadel)
        .unwrap_or_else(|| panic!("`{nadel}` fehlt"))
}

/// Der Rumpf der Funktion `name` (bis zur nächsten Funktion auf oberster Ebene).
fn rumpf(s: &str, name: &str) -> String {
    let s = ohne_tests(s);
    let start = s
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("fn {name} fehlt"));
    let rest = &s[start..];
    let ende = rest[1..].find("\n}\n").map_or(rest.len(), |e| e + 3);
    rest[..ende].to_string()
}

#[test]
fn chat_zweigt_vor_dem_notizbuchschutz_ab() {
    let r = rumpf(MOD, "voice_call_chat");
    let zweig = r.find("projekt::bindung(").expect("Projektzweig fehlt");
    let schutz = r.find("schutz_pruefen").expect("Schutz fehlt");
    let kontext = r.find("bind_for_speaker_quellen").expect("Kontext fehlt");
    assert!(zweig < schutz, "Projektzweig vor schutz_pruefen");
    assert!(zweig < kontext, "Projektzweig vor dem App-Kontext");
    // Der Zweig kehrt zurück, bevor etwas vom gewöhnlichen Weg läuft.
    assert!(r[zweig..schutz].contains("return projekt::chat("));
}

#[test]
fn durchgang_zweigt_vor_dem_notizbuchschutz_ab() {
    let r = rumpf(TURN, "voice_call_turn");
    let zweig = r.find("projekt::bindung(").expect("Projektzweig fehlt");
    let schutz = r.find("schutz_pruefen").expect("Schutz fehlt");
    assert!(zweig < schutz, "Projektzweig vor schutz_pruefen");
    assert!(r[zweig..schutz].contains("projekt::durchgang("));
    // Ein Abbruch vor dem Start wird weiterhin zuerst eingelöst.
    assert!(r.find("EARLY_CANCELS").unwrap() < zweig);
}

#[test]
fn projektzweig_kennt_weder_app_kontext_noch_notizbuch_noch_werkzeuge() {
    let p = ohne_tests(PROJEKT);
    for verboten in [
        "bind_for_speaker_quellen",
        "schutz_pruefen",
        "werkzeug_register",
        "voice_tool_call",
        "SpeakerContext",
        "notizbuch",
    ] {
        // Erwähnungen im Kommentar zählen nicht: gesucht wird im Code ohne `//`-Zeilen.
        let code = code_ohne_kommentare(&p);
        assert!(!code.contains(verboten), "`{verboten}` im Projektzweig");
    }
    // Das Ausgabetor des Orchestrators erhält keine Werkzeuge.
    assert!(p.contains("StromTor::neu(sprache, quellen, Vec::new())"));
}

#[test]
fn versand_steht_hinter_dem_tor_und_vor_dem_abschluss() {
    let p = ohne_tests(PROJEKT);
    let r = rumpf(PROJEKT, "durchlauf");
    let _ = p;
    let vor = r.find(".vor_versand(").expect("vor_versand fehlt");
    let senden = r.find("weg.senden(").expect("Versand fehlt");
    let abschluss = r.find(".abschliessen(").expect("abschliessen fehlt");
    let select = r.find("tokio::select!").expect("select fehlt");
    assert!(vor < senden, "vor_versand vor dem Versand");
    assert!(senden < abschluss, "Versand vor dem Abschluss");
    assert!(select < abschluss);
    // Widerruf (Tor) und Abbruch (Oberfläche) stehen im select, der Widerruf zuerst.
    let sel = &r[select..abschluss];
    assert!(sel.contains("biased;"));
    assert!(sel.find("abbruch.notified()").unwrap() < sel.find("cancel.notified()").unwrap());
    // Nichts geht an die Oberfläche, bevor das Tor geurteilt hat: `ausliefern` steht erst im Zweig `Ok`.
    let ausliefern = r.find("ausliefern(").expect("ausliefern fehlt");
    assert!(abschluss < ausliefern, "Lieferung nach dem Urteil des Tors");
    // Es gibt keinen direkten Aufruf des Sendewegs am Tor vorbei.
    assert!(!p.contains("send_chat_completion_stream(\n"));
    assert_eq!(
        p.matches("send_chat_completion_stream_routed(").count(),
        1,
        "genau ein Sendeweg, in LlmWeg"
    );
}

#[test]
fn lokaler_versand_nur_ueber_die_gebundene_adresse() {
    let code = code_ohne_kommentare(&ohne_tests(PROJEKT));
    // `senden` gibt es zweimal: im Merkmal (ohne Rumpf) und im echten Weg `LlmWeg`.
    let weg = code
        .find("impl Modellweg for LlmWeg")
        .expect("LlmWeg fehlt");
    let w = &code[weg..];
    let adresse = w.find("sendeadresse(ziel)").expect("Adresse fehlt");
    let senden = w
        .find("send_chat_completion_stream_routed(")
        .expect("Sendeweg fehlt");
    assert!(adresse < senden, "die Adresse steht vor dem Versand fest");
    assert!(
        w[senden..].contains("adresse,"),
        "die gebundene Adresse geht an den Sendeweg"
    );
    assert!(
        !code.contains("base_url"),
        "die base_url gilt für Projektdaten nie"
    );
}

#[test]
fn werkzeugaufruf_im_projekt_wird_vor_allem_anderen_verweigert() {
    let r = rumpf(TOOLS, "voice_tool_call");
    let pruefung = r.find("werkzeuge_erlaubt(").expect("Prüfung fehlt");
    let erste_arbeit = r.find("parts(&app)").expect("parts fehlt");
    let ausfuehrung = r.find("voice::call(").expect("Ausführung fehlt");
    assert!(pruefung < erste_arbeit);
    assert!(pruefung < ausfuehrung);
    assert!(r.find("LIMITER").unwrap() > pruefung);
}

#[test]
fn loeschen_eines_gespraechs_ruft_gespraech_vergessen() {
    let r = rumpf(CONVERSATIONS, "conversation_delete");
    assert!(r.contains("project_binding"));
    assert!(r.contains("projekt::gespraech_vergessen"));
    let g = rumpf(PROJEKT, "gespraech_vergessen");
    assert!(g.contains("gespraech_geloescht("));
    assert!(g.contains(".gespraech_vergessen("));
}

#[test]
fn widerruf_trifft_nur_genannte_anfragen() {
    let r = rumpf(ZUSTAND, "widerruf_melden");
    assert!(r.contains("revoke_requests(app, ids)"));
    assert!(
        !r.contains("revoke_context_requests"),
        "kein Widerruf aller Anfragen mit Kontext"
    );
    // Das Ereignis `*` (alle) kommt im gezielten Widerruf nicht vor.
    let g = rumpf(MOD, "revoke_requests");
    assert!(!g.contains("\"*\""));
}

#[test]
fn gespraech_im_projekt_wird_nur_beim_anlegen_gebunden() {
    let p = ohne_tests(include_str!("../conversations/store.rs"));
    // `project_id` wird nur beim Anlegen gesetzt: Kein UPDATE auf `scope`.
    assert!(!p.contains("UPDATE conversations SET scope"));
    assert!(!p.contains("scope = "));
    let s = stelle(include_str!("../conversations/store.rs"), "fn create_in(");
    assert!(s > 0);
}

#[test]
fn ipc_waehlt_nur_aus_und_das_tor_bestimmt_das_ziel() {
    let r = rumpf(PROJEKT, "durchlauf");
    assert!(r.contains("modell: a.modell.as_deref()"));
    assert!(r.contains("erwarteter_stand: a.erwarteter_stand.as_deref()"));
    assert!(r.contains("stufe_zulaessig(s, &ziel)"));
    // Das Ziel kommt aus der Freigabe des Tors, nie aus der Anfrage.
    assert!(!r.contains("a.ziel") && !r.contains("a.anbieter") && !r.contains("a.provider"));
    // Die Souveränitätsstufe prüft der Durchgang vorab (Länder der Verbindung), vor dem Tor.
    let t = rumpf(TURN, "voice_call_turn");
    let zweig = t.find("projekt::bindung(").expect("Projektzweig fehlt");
    let pruefung = t[zweig..].find("resolve(").expect("Stufe vorab fehlt") + zweig;
    let durchgang = t.find("projekt::durchgang(").expect("Durchgang fehlt");
    assert!(zweig < pruefung && pruefung < durchgang);
}

#[test]
fn nichts_wird_vor_dem_urteil_des_tors_gemeldet() {
    let r = rumpf(PROJEKT, "durchlauf");
    let on_delta = r.find("let on_delta").expect("on_delta fehlt");
    let select = r.find("tokio::select!").expect("select fehlt");
    let kopf = &r[on_delta..select];
    // Der Rückruf je Textstück puffert nur: keine Meldung an die Oberfläche.
    assert!(!kopf.contains("melden("), "on_delta meldet nichts");
    assert!(!kopf.contains("emit("));
    // Die Meldungen des Erfolgs stehen in `ausliefern`, aufgerufen erst nach `abschliessen`.
    let a = rumpf(PROJEKT, "ausliefern");
    assert!(a.contains("\"sentence\"") && a.contains("\"done\""));
    assert_eq!(ohne_tests(PROJEKT).matches("ausliefern(bote").count(), 1);
}
