//! Zusatzvektoren der Ablage für den iOS-Port (D5, Epic #395, lva#441): Fälle, die der Grundsatz in
//! `ablage_vektoren.rs` nicht enthält. Sie stehen unter dem Schlüssel `zusatz` derselben Datei
//! `docs/superpowers/plans/ablage-testvektoren.json`; Swift (`VoiceCore`) liest sie von dort.
//!
//! Alle Werte entstehen aus den echten Desktop-Funktionen (`collect::canonical`, serde-Lesen von `Datei` und
//! `Ordner`, `ablage::id_gueltig`, `zusammenfuehren_datei`/`_ordner`, `hash_datei`, `hub_blob`, `grab_nutzlast`,
//! der JSON-Leser von serde_json), nicht aus Literalen. `json_passt_zum_code` vergleicht die Datei mit dem Code;
//! mit `LVA_VEKTOREN_SCHREIBEN=1` schreibt derselbe Test den Teil `zusatz` neu (die übrigen Teile bleiben).
//!
//! Alle Schlüssel, Kennungen und Namen sind synthetisch (Schlüssel `00 01 … 1f`, Benutzer 7).

use super::vektoren::json_abgleichen;
use super::*;
use crate::ablage::zusammenfuehren::{Fassung, Text, Wahl};
use serde_json::json;
use std::collections::BTreeMap;

const USER: i64 = 7;
const G1: &str = "g_01HZX3K9M2N4P5Q6R7S8T9V0W1";
const G2: &str = "g_01HZX3K9M2N4P5Q6R7S8T9V0W5";
const ORD: &str = "ord_01HZX3K9M2N4P5Q6R7S8T9V0WX";
const ORD_B: &str = "ord_01HZX3K9M2N4P5Q6R7S8T9V0WZ";
const ORD_E: &str = "ord_01HZX3K9M2N4P5Q6R7S8T9V0WW";
const DAT: &str = "dat_01HZX3K9M2N4P5Q6R7S8T9V0WY";
const FAS_1: &str = "fas_01HZX3K9M2N4P5Q6R7S8T9V0W2";
const FAS_2: &str = "fas_01HZX3K9M2N4P5Q6R7S8T9V0W3";
const FAS_3: &str = "fas_01HZX3K9M2N4P5Q6R7S8T9V0W6";
const FAS_X: &str = "fas_01HZX3K9M2N4P5Q6R7S8T9V0WA";
const FAS_Z: &str = "fas_01HZX3K9M2N4P5Q6R7S8T9V0WB";
const WAHL_1: &str = "wahl_01HZX3K9M2N4P5Q6R7S8T9V0W4";
const WAHL_2: &str = "wahl_01HZX3K9M2N4P5Q6R7S8T9V0W7";
const SHA_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SHA_2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const SHA_3: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const SHA_4: &str = "4444444444444444444444444444444444444444444444444444444444444444";

fn schluessel() -> [u8; KEY_LEN] {
    let mut k = [0u8; KEY_LEN];
    for (i, b) in k.iter_mut().enumerate() {
        *b = i as u8;
    }
    k
}

fn reg<T>(wert: T, zaehler: u64, geraet: &str) -> Register<T> {
    Register::neu(wert, zaehler, geraet)
}

fn extra(json_text: &str) -> BTreeMap<String, Value> {
    serde_json::from_str(json_text).expect("extra")
}

fn text(rev: &str, blob: &str, stand: &str) -> Text {
    Text {
        textrev: rev.to_string(),
        blob: blob.to_string(),
        size: 420,
        extraktor: "lva-text".to_string(),
        extraktor_version: "1".to_string(),
        marken_version: 1,
        stand: stand.to_string(),
        code: None,
        versuche: 0,
        versuche_zeit: 0,
    }
}

fn text_fehler(rev: &str) -> Text {
    let mut t = text(rev, "", "fehler");
    t.code = Some("extrakt.zeit".to_string());
    t.versuche = 2;
    t.versuche_zeit = 1;
    t
}

fn fassung(
    fas: &str,
    vorg: Option<&str>,
    sha: &str,
    size: u64,
    zeit: i64,
    texte: Vec<Text>,
) -> Fassung {
    Fassung {
        fas: fas.to_string(),
        vorgaenger: vorg.map(str::to_string),
        blob: sha.to_string(),
        size,
        original_sha256: sha.to_string(),
        typ: "txt".to_string(),
        geraet: G1.to_string(),
        zeit,
        texte,
    }
}

fn wahl(
    id: &str,
    gewaehlt: &str,
    gegen: &[&str],
    ersetzt: &[&str],
    zaehler: u64,
    geraet: &str,
) -> Wahl {
    Wahl {
        id: id.to_string(),
        gewaehlt: gewaehlt.to_string(),
        gegen: gegen.iter().map(|s| s.to_string()).collect(),
        ersetzt: ersetzt.iter().map(|s| s.to_string()).collect(),
        zaehler,
        geraet: geraet.to_string(),
    }
}

fn datei_leer() -> Datei {
    Datei {
        id: DAT.to_string(),
        ordner: reg(ORD.to_string(), 3, G1),
        name: reg("Mietvertrag Köln.txt".to_string(), 2, G1),
        klasse_min: reg("public".to_string(), 1, G1),
        nur_geraet_min: false,
        fassungen: vec![],
        wahlen: vec![],
        entfernt: vec![],
        blob_refs: vec![],
        papierkorb: reg(None, 1, G1),
        extra: BTreeMap::new(),
    }
}

fn kanon_datei(d: &Datei) -> String {
    collect::canonical(&wert_datei(d))
}

fn kanon_ordner(o: &Ordner) -> String {
    collect::canonical(&wert_ordner(o))
}

/// Form `{name, ein, gueltig, aus}`: `aus` ist die kanonische Form, die serde_json und
/// `collect::canonical` aus `ein` machen, oder `null`, wenn serde_json `ein` ablehnt.
fn roh_faelle(faelle: &[(String, String)]) -> Vec<Value> {
    faelle
        .iter()
        .map(|(name, ein)| {
            let aus = serde_json::from_str::<Value>(ein)
                .ok()
                .map(|v| collect::canonical(&v));
            json!({"name": name, "ein": ein, "gueltig": aus.is_some(), "aus": aus})
        })
        .collect()
}

fn paare(l: &[(&str, &str)]) -> Vec<(String, String)> {
    l.iter()
        .map(|(n, e)| (n.to_string(), e.to_string()))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Gruppen
// ---------------------------------------------------------------------------------------------

fn escapes() -> Value {
    let alle_steuer = format!(
        "\"{}\"",
        (0..0x20u32)
            .map(|c| format!("\\u{c:04x}"))
            .collect::<String>()
    );
    let mut f = paare(&[
        (
            "steuerzeichen_und_umlaute",
            r#"{"s":"a\u0001b\u001f\u007f\"\\/\n\t\r\b\fé😀   end","ké":"v"}"#,
        ),
        ("schraegstrich_nicht_escaped", r#"{"a\/b":"c\/d/e"}"#),
        ("nicht_ascii_roh", r#"{"k":"Größe – 日本語 😀"}"#),
        // Rohtext mit JSON-Escapes (nicht die Zeichen selbst): U+1F600 als Ersatzpaar, Hexziffern gross/klein.
        ("ersatzpaar", r#""\ud83d\ude00""#),
        ("ersatzpaar_grossbuchstaben", r#""\uD83D\uDE00""#),
        ("hexziffern_grossbuchstaben", r#""\u00E9\u00DC""#),
        ("hexziffern_kleinbuchstaben", r#""\u00e9\u00dc""#),
        ("hexziffern_gemischt", r#""\u00E9\u00dc\uFfE0""#),
        // Schlüssel als Escape: Ersatzpaar, zusammengesetzt und zerlegt (zwei verschiedene Schlüssel).
        ("schluessel_ersatzpaar_escape", r#"{"\ud83d\ude00":1}"#),
        (
            "schluessel_zusammengesetzt_zerlegt_escape",
            r#"{"\u00e9":1,"e\u0301":2}"#,
        ),
        ("schluessel_gleich_roh_und_escape", r#"{"\u00e9":1,"é":2}"#),
        ("leerraum", " {\n \"b\" : [ 1 , 2 ] ,\t\"a\" : { } }\r\n"),
        (
            "verschachtelt",
            "[ {\"b\" : {\"d\":null,\"c\":[true, false,null,{\"z\":1,\"y\":[]}]},\n\"a\":{}} , \"x\" ]",
        ),
        ("null_bleibt", r#"{"a":null,"b":[],"c":{},"d":[null,{"e":null}]}"#),
        // Ablehnung
        ("steuerzeichen_roh_ungueltig", "\"a\u{1}b\""),
        (
            "einzelnes_fuehrendes_ersatzzeichen_ungueltig",
            r#""\ud800""#,
        ),
        (
            "einzelnes_nachfolgendes_ersatzzeichen_ungueltig",
            r#""\udc00""#,
        ),
        ("ersatzzeichen_ohne_paar_ungueltig", r#""\ud800A""#),
        ("unbekannte_escape_ungueltig", r#""\x""#),
        ("kurze_u_escape_ungueltig", r#""\u12""#),
        ("bom_ungueltig", "\u{feff}1"),
        ("nachlauf_ungueltig", "[1] x"),
        ("komma_am_ende_ungueltig", "[1,]"),
        ("objekt_komma_am_ende_ungueltig", r#"{"a":1,}"#),
        ("einfache_anfuehrungszeichen_ungueltig", "'a'"),
        ("leer_ungueltig", ""),
        ("unvollstaendig_ungueltig", "{\"a\":"),
    ]);
    f.push(("alle_steuerzeichen".to_string(), alle_steuer));
    json!({"faelle": roh_faelle(&f)})
}

fn zahlen() -> Value {
    let gueltig = [
        "1.5",
        "0.1",
        "1e21",
        "1e-7",
        "0.00001",
        "123456789.125",
        "1e16",
        "1e15",
        "1e20",
        "12345678901234567890123",
        "-2.5e-10",
        "5e-324",
        "1.7976931348623157e308",
        "100.0",
        "1E2",
        "-0",
        "-0.0",
        "0.5",
        "123456.789e3",
        "1e-4",
        "9007199254740993",
        "18446744073709551615",
        "18446744073709551616",
        "-9223372036854775808",
        "-9223372036854775809",
        "0",
        "-1",
        "1e0",
        "123456789012345678.0",
        "0.000123",
        "1.5e300",
        "2.5e-5",
        "0.30000000000000004",
        "1.0e+2",
        "-1e-7",
        "[1.0,-0.0,1e20,0.1]",
        // Grenzen der Schreibweise: ab 1e-5 dezimal, darunter Exponent; ab 1e16 Exponent.
        "1e-5",
        "1e-6",
        "1.5e-6",
        "9.99e-7",
        "9007199254740992.0",
        "1e22",
        "4.35",
    ];
    let ungueltig = [
        "01", "1.", ".5", "+1", "-", "1e", "1e+", "1e999", "--1", "1.e5", "0x10", "NaN",
        "Infinity", "1_000", "- 1", "1 2",
    ];
    let f: Vec<(String, String)> = gueltig
        .iter()
        .chain(ungueltig.iter())
        .map(|z| (z.to_string(), z.to_string()))
        .collect();
    // JSON kennt weder NaN noch Unendlich: serde_json schreibt `null`.
    let nicht_endlich: Vec<Value> = [
        ("unendlich", f64::INFINITY),
        ("minus_unendlich", f64::NEG_INFINITY),
        ("nan", f64::NAN),
    ]
    .iter()
    .map(|(n, x)| json!({"name": n, "aus": collect::canonical(&Value::from(*x))}))
    .collect();
    json!({"faelle": roh_faelle(&f), "nicht_endlich": nicht_endlich})
}

fn umlaut_schluessel() -> Value {
    let f = paare(&[
        (
            "umlaute_und_grossbuchstaben",
            r#"{"zebra":1,"Zebra":2,"Äpfel":3,"Äpfel":4,"äpfel":5,"a":6}"#,
        ),
        (
            "zusammengesetzt_und_zerlegt",
            r#"{"é":1,"é":2,"B":3,"a":4}"#,
        ),
        (
            "leer_emoji_und_grenzen",
            r#"{"":0,"😀":1,"~":2,"\u007f":3,"\u0080":4}"#,
        ),
        // UTF-8-Bytereihenfolge weicht von UTF-16 ab: U+FFE0 (EF BF A0) kommt vor U+1F600 (F0 9F 98 80), in UTF-16
        // steht D83D DE00 vor FFE0. Roh und als Escape.
        (
            "bmp_ueber_ersatzbereich_roh",
            "{\"\u{1F600}\":1,\"\u{FFE0}\":2,\"\u{E000}\":3,\"a\":4}",
        ),
        (
            "bmp_ueber_ersatzbereich_escape",
            r#"{"\ud83d\ude00":1,"\uffe0":2,"\ue000":3,"a":4}"#,
        ),
        (
            "schluessel_mit_escape",
            r#"{"a\"b":1,"a\\b":2,"a\nb":3,"a/b":4}"#,
        ),
        (
            "doppelte_schluessel_letzter_gewinnt",
            r#"{"a":1,"a":2,"b":{"c":1,"c":null}}"#,
        ),
    ]);
    let o1 = Ordner {
        id: ORD.to_string(),
        eltern: reg(None, 3, G1),
        name: reg("Verträge".to_string(), 2, G1),
        datenklasse: reg("erbt".to_string(), 1, G1),
        nur_geraet: reg(false, 1, G1),
        zielordner_ki: reg(true, 1, G2),
        papierkorb: reg(None, 1, G1),
        extra: extra(
            r#"{"v":1,"zukunft":{"b":[],"a":{}},"zebra":1,"Zebra":2,"Äpfel":3,"Äpfel":4,"äpfel":5,"a":6}"#,
        ),
    };
    let o2 = Ordner {
        id: ORD.to_string(),
        eltern: reg(Some(ORD_E.to_string()), 7, G2),
        name: reg("Rechnungen 2026".to_string(), 6, G2),
        datenklasse: reg("internal".to_string(), 5, G2),
        nur_geraet: reg(true, 4, G1),
        zielordner_ki: reg(false, 1, G1),
        papierkorb: reg(Some(1_762_800_000_000), 8, G2),
        extra: BTreeMap::new(),
    };
    let nutzlasten: Vec<Value> = [
        ("ordner_mit_umlaut_schluesseln_und_v1", &o1),
        ("ordner_ohne_null", &o2),
    ]
    .iter()
    .map(|(n, o)| {
        json!({"name": n, "serde_roh": serde_json::to_string(o).unwrap(),
               "kanonisch": kanon_ordner(o), "hash": hash_ordner(o)})
    })
    .collect();
    json!({"faelle": roh_faelle(&f), "nutzlasten": nutzlasten})
}

/// Dokumente, die der Desktop (`entschluesseln`: `serde_json::from_value`) annimmt oder ablehnt. Die erwartete
/// Annahme steht hier nur als Absicht; das Ergebnis in der Datei kommt aus dem echten Lesen.
fn serde_lesefaelle() -> Value {
    let r = |w: &str| format!(r#"{{"wert":{w},"zaehler":1,"geraet":"g_x"}}"#);
    let mit_zaehler = |z: &str| r("\"n\"").replace("\"zaehler\":1", &format!("\"zaehler\":{z}"));
    let ordner_json = |eltern: String, name: String, nur: String, pk: String, extra: &str| {
        format!(
            r#"{{"id":"{ORD}","eltern":{eltern},"name":{name},"datenklasse":{dk},"nur_geraet":{nur},"zielordner_ki":{zk},"papierkorb":{pk}{extra}}}"#,
            dk = r("\"erbt\""),
            zk = r("false"),
        )
    };
    let o_basis = || ordner_json(r("null"), r("\"n\""), r("false"), r("null"), "");
    let datei_json = |fass: &str, nur: &str, extra: &str| {
        format!(
            r#"{{"id":"{DAT}","ordner":{o},"name":{n},"klasse_min":{k}{nur}{fass},"papierkorb":{pk}{extra}}}"#,
            o = r(&format!("\"{ORD}\"")),
            n = r("\"a.txt\""),
            k = r("\"public\""),
            pk = r("null"),
        )
    };
    let fas_json = |zeit: &str, size: &str, vorg: &str, texte: &str| {
        format!(
            r#"{{"fas":"{FAS_1}"{vorg},"blob":"{SHA_1}","size":{size},"original_sha256":"{SHA_1}","typ":"txt","geraet":"g_x","zeit":{zeit}{texte}}}"#
        )
    };
    let text_json = |marken: &str, versuche: &str, code: &str| {
        format!(
            r#"{{"textrev":"aa","blob":"{SHA_3}","size":5,"extraktor":"lva-text","extraktor_version":"1","marken_version":{marken},"stand":"ok"{code}{versuche}}}"#
        )
    };
    let mit_fassung = |zeit: &str, size: &str, vorg: &str, texte: &str| {
        datei_json(
            &format!(",\"fassungen\":[{}]", fas_json(zeit, size, vorg, texte)),
            "",
            "",
        )
    };
    let mit_text = |marken: &str, versuche: &str, code: &str| {
        mit_fassung(
            "1",
            "1",
            "",
            &format!(",\"texte\":[{}]", text_json(marken, versuche, code)),
        )
    };
    let wahl_json = |gegen: &str, zaehler: &str| {
        datei_json(
            &format!(
                ",\"wahlen\":[{{\"id\":\"{WAHL_1}\",\"gewaehlt\":\"{FAS_1}\"{gegen}{zaehler},\"geraet\":\"g_x\"}}]"
            ),
            "",
            "",
        )
    };
    let o = |eltern: String, name: String, nur: String, pk: String| {
        ordner_json(eltern, name, nur, pk, "")
    };
    let faelle: Vec<(&str, String, bool)> = vec![
        ("o_basis", o_basis(), true),
        (
            "o_eltern_wert_fehlt",
            o(
                r#"{"zaehler":1,"geraet":"g_x"}"#.into(),
                r("\"n\""),
                r("false"),
                r("null"),
            ),
            true,
        ),
        (
            "o_eltern_wert_string",
            o(r("\"ord_x\""), r("\"n\""), r("false"), r("null")),
            true,
        ),
        (
            "o_name_wert_fehlt",
            o(
                r("null"),
                r#"{"zaehler":1,"geraet":"g_x"}"#.into(),
                r("false"),
                r("null"),
            ),
            false,
        ),
        (
            "o_name_wert_null",
            o(r("null"), r("null"), r("false"), r("null")),
            false,
        ),
        (
            "o_nur_geraet_string",
            o(r("null"), r("\"n\""), r("\"false\""), r("null")),
            false,
        ),
        (
            "o_papierkorb_float",
            o(r("null"), r("\"n\""), r("false"), r("1.5")),
            false,
        ),
        (
            "o_papierkorb_negativ",
            o(r("null"), r("\"n\""), r("false"), r("-5")),
            true,
        ),
        (
            "o_papierkorb_zu_gross",
            o(r("null"), r("\"n\""), r("false"), r("9223372036854775808")),
            false,
        ),
        (
            "o_papierkorb_max",
            o(r("null"), r("\"n\""), r("false"), r("9223372036854775807")),
            true,
        ),
        (
            "o_zaehler_float",
            o(r("null"), mit_zaehler("1.0"), r("false"), r("null")),
            false,
        ),
        (
            "o_zaehler_negativ",
            o(r("null"), mit_zaehler("-1"), r("false"), r("null")),
            false,
        ),
        (
            "o_zaehler_max",
            o(
                r("null"),
                mit_zaehler("18446744073709551615"),
                r("false"),
                r("null"),
            ),
            true,
        ),
        (
            "o_zaehler_zu_gross",
            o(
                r("null"),
                mit_zaehler("18446744073709551616"),
                r("false"),
                r("null"),
            ),
            false,
        ),
        (
            "o_register_zusatzfeld",
            o(r("null"), mit_zaehler("1,\"neu\":7"), r("false"), r("null")),
            true,
        ),
        (
            "o_ohne_id",
            o_basis().replace(&format!("\"id\":\"{ORD}\","), ""),
            false,
        ),
        (
            "o_id_zahl",
            o_basis().replace(&format!("\"id\":\"{ORD}\""), "\"id\":5"),
            false,
        ),
        (
            "o_extra_v1",
            ordner_json(
                r("null"),
                r("\"n\""),
                r("false"),
                r("null"),
                ",\"v\":1,\"x\":null",
            ),
            true,
        ),
        ("o_kein_objekt", "[1,2]".to_string(), false),
        ("d_minimal", datei_json("", "", ""), true),
        (
            "d_nur_geraet_null",
            datei_json("", ",\"nur_geraet_min\":null", ""),
            false,
        ),
        (
            "d_nur_geraet_true",
            datei_json("", ",\"nur_geraet_min\":true", ""),
            true,
        ),
        (
            "d_fassungen_null",
            datei_json(",\"fassungen\":null", "", ""),
            false,
        ),
        (
            "d_fassung_ok",
            mit_fassung("1730000000123", "10", ",\"vorgaenger\":null", ""),
            true,
        ),
        (
            "d_fassung_vorgaenger_fehlt",
            mit_fassung("1", "10", "", ""),
            true,
        ),
        (
            "d_fassung_zeit_float",
            mit_fassung("1.5", "10", "", ""),
            false,
        ),
        (
            "d_fassung_zeit_negativ",
            mit_fassung("-1", "10", "", ""),
            true,
        ),
        (
            "d_fassung_size_negativ",
            mit_fassung("1", "-1", "", ""),
            false,
        ),
        (
            "d_fassung_size_max",
            mit_fassung("1", "18446744073709551615", "", ""),
            true,
        ),
        (
            "d_fassung_texte_null",
            mit_fassung("1", "1", "", ",\"texte\":null"),
            false,
        ),
        ("d_text_ok", mit_text("1", "", ""), true),
        (
            "d_text_versuche_null",
            mit_text("1", ",\"versuche\":null", ""),
            false,
        ),
        (
            "d_text_code_null",
            mit_text("1", "", ",\"code\":null"),
            true,
        ),
        ("d_text_code_zahl", mit_text("1", "", ",\"code\":5"), false),
        (
            "d_text_marken_zu_gross",
            mit_text("4294967296", "", ""),
            false,
        ),
        (
            "d_text_versuche_max",
            mit_text("4294967295", ",\"versuche\":4294967295", ""),
            true,
        ),
        ("d_wahl_gegen_fehlt", wahl_json("", ",\"zaehler\":3"), true),
        (
            "d_wahl_gegen_null",
            wahl_json(",\"gegen\":null", ",\"zaehler\":3"),
            false,
        ),
        ("d_wahl_ohne_zaehler", wahl_json("", ""), false),
        (
            "d_blob_refs_string",
            datei_json(",\"blob_refs\":\"x\"", "", ""),
            false,
        ),
        (
            "d_blob_refs_liste",
            datei_json(",\"blob_refs\":[\"a\",\"b\"]", "", ""),
            true,
        ),
        (
            "d_entfernt_zahl",
            datei_json(",\"entfernt\":[1]", "", ""),
            false,
        ),
        (
            "d_ohne_klasse_min",
            datei_json("", "", "").replace(&format!(",\"klasse_min\":{}", r("\"public\"")), ""),
            false,
        ),
        (
            "d_ohne_papierkorb",
            datei_json("", "", "").replace(&format!(",\"papierkorb\":{}", r("null")), ""),
            false,
        ),
        (
            "d_extra_zahlen",
            datei_json(
                "",
                "",
                ",\"x\":1.0,\"y\":18446744073709551615,\"z\":[null,{\"q\":-0}]",
            ),
            true,
        ),
    ];
    let ausgabe: Vec<Value> = faelle
        .iter()
        .map(|(name, t, absicht)| {
            let v: Value = serde_json::from_str(t).expect("Fall ist gültiges JSON");
            let datei = name.starts_with("d_");
            let ok = if datei {
                serde_json::from_value::<Datei>(v).is_ok()
            } else {
                serde_json::from_value::<Ordner>(v).is_ok()
            };
            assert_eq!(ok, *absicht, "Fall {name}");
            json!({"name": name, "art": if datei { "datei" } else { "ordner" }, "json": t, "ok": ok})
        })
        .collect();

    // Schema-Marke `v` wie `entschluesseln`: nur eine nichtnegative Ganzzahl zählt, sonst gilt 1; `v` > SCHEMA
    // lässt das Objekt ruhen.
    let schema: Vec<Value> = [
        r#"{}"#,
        r#"{"v":1}"#,
        r#"{"v":0}"#,
        r#"{"v":2}"#,
        r#"{"v":99}"#,
        r#"{"v":18446744073709551615}"#,
        r#"{"v":"2"}"#,
        r#"{"v":2.0}"#,
        r#"{"v":-1}"#,
        r#"{"v":null}"#,
        r#"{"v":true}"#,
    ]
    .iter()
    .map(|t| {
        let v: Value = serde_json::from_str(t).unwrap();
        let gelesen = v.get("v").and_then(Value::as_u64).unwrap_or(1);
        json!({"json": t, "gelesen": gelesen, "akzeptiert": gelesen <= SCHEMA})
    })
    .collect();
    json!({"faelle": ausgabe, "schema_marke": schema})
}

fn id_gueltig_faelle() -> Value {
    let ids: &[(&str, &str)] = &[
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0TA"),
        ("dat_", "dat_01hzx4q8e5k2v7m3n9p6r1s0ta"),
        ("dat_", "dat_8ZZZZZZZZZZZZZZZZZZZZZZZZZ"),
        ("dat_", "dat_ZZZZZZZZZZZZZZZZZZZZZZZZZZ"),
        ("dat_", "dat_00000000000000000000000000"),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0T"),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0TAA"),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0TI"),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0TL"),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0TO"),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0TU"),
        ("dat_", "dat_01hzx4q8e5k2v7m3n9p6r1s0ti"),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0T/"),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0T."),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0T\n"),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0T\u{e4}"),
        ("dat_", "dat_../../../../etc/passwd_______"),
        ("dat_", "ord_01HZX4Q8E5K2V7M3N9P6R1S0TA"),
        ("dat_", "DAT_01HZX4Q8E5K2V7M3N9P6R1S0TA"),
        ("dat_", "01HZX4Q8E5K2V7M3N9P6R1S0TA"),
        ("dat_", ""),
        ("ord_", "ord_01HZX4Q8E5K2V7M3N9P6R1S0TB"),
        ("g_", "g_01HZX4Q8E5K2V7M3N9P6R1S0T1"),
        ("fas_", "fas_01HZX4Q8E5K2V7M3N9P6R1S0TD"),
        ("wahl_", "wahl_01HZX4Q8E5K2V7M3N9P6R1S0TF"),
    ];
    let faelle: Vec<Value> = ids
        .iter()
        .map(|(praefix, id)| {
            json!({"praefix": praefix, "id": id, "gueltig": crate::ablage::id_gueltig(praefix, id)})
        })
        .collect();
    json!({"faelle": faelle})
}

/// Größte Schachtelung, die serde_json noch liest (`recursion_limit`).
fn tiefe_127() -> Value {
    let max = |auf: &dyn Fn(usize) -> String| {
        (1..300usize)
            .filter(|n| serde_json::from_str::<Value>(&auf(*n)).is_ok())
            .max()
            .unwrap_or(0)
    };
    let array = max(&|n| format!("{}{}", "[".repeat(n), "]".repeat(n)));
    let objekt = max(&|n| format!("{}1{}", "{\"a\":".repeat(n), "}".repeat(n)));
    json!({"max_tiefe_array": array, "max_tiefe_objekt": objekt})
}

/// Hex-Hash → Hub-Kennung (`hub_blob`): Großbuchstaben und `+x`-Paare nimmt Rusts `from_str_radix` an.
fn hub_hex() -> Value {
    let key = schluessel();
    let sha = blobs::sha256_hex(b"Hallo Buch");
    let eingaben: Vec<(&str, String)> = vec![
        ("klein", sha.clone()),
        ("gross", sha.to_uppercase()),
        ("plus_paar", format!("+1{}", &sha[2..])),
        ("zu_kurz", sha[..63].to_string()),
        ("zu_lang", format!("{sha}00")),
        ("kein_hex", format!("zz{}", &sha[2..])),
        ("minus_paar", format!("-1{}", &sha[2..])),
        ("nicht_ascii_paar", format!("\u{e9}{}", &sha[2..])),
        ("leer", String::new()),
    ];
    let faelle: Vec<Value> = eingaben
        .iter()
        .map(|(n, e)| json!({"name": n, "eingabe": e, "hub_kennung": hub_blob(&key, e)}))
        .collect();
    json!({"faelle": faelle})
}

/// Grabsteine anderer Benutzer und Sammlungen (die Grundmenge hat nur Benutzer 7).
fn grabstein_benutzer() -> Value {
    let key = schluessel();
    let kid = crypto::key_id(&key);
    let faelle: Vec<Value> = [
        (8i64, "ablage_datei", DAT),
        (8, "ablage_ordner", ORD),
        (-7, "ablage_datei", DAT),
    ]
    .iter()
    .map(|(user, coll, id)| {
        let payload = grab_nutzlast(&key, *user, coll, id).unwrap();
        assert!(grab_echt(&key, *user, coll, id, &kid, &payload));
        json!({"user_id": user, "collection": coll, "object_id": id, "key_id": kid,
               "payload_base64": payload})
    })
    .collect();
    json!({"faelle": faelle})
}

// --- Zusammenführen -----------------------------------------------------------------------------

fn merge_a() -> Datei {
    let mut d = datei_leer();
    d.name = reg("Vertrag alt.pdf".to_string(), 2, G1);
    d.fassungen = vec![
        fassung(
            FAS_1,
            None,
            SHA_1,
            1234,
            1_762_700_000_000,
            vec![text("aa", SHA_3, "ok")],
        ),
        fassung(FAS_2, Some(FAS_1), SHA_2, 99, 1_762_700_100_000, vec![]),
    ];
    d.wahlen = vec![wahl(WAHL_1, FAS_2, &[FAS_1], &[], 4, G1)];
    d.extra = extra(r#"{"zukunft":{"a":1},"nur_a":1}"#);
    d
}

fn merge_b() -> Datei {
    let mut d = datei_leer();
    d.ordner = reg(ORD_B.to_string(), 3, G2); // gleicher Zähler, größeres Gerät: b gewinnt
    d.name = reg("Vertrag neu.pdf".to_string(), 5, G2); // höherer Zähler: b gewinnt
    d.klasse_min = reg("internal".to_string(), 1, G1); // gleicher Rang, größerer Wert: a ("public") gewinnt
    d.nur_geraet_min = true; // ODER
    d.fassungen = vec![
        // dieselbe Fassung: Text `aa` als Fehler (a hat ihn mit blob, das ist größer) plus neuer Text `bb`
        fassung(
            FAS_1,
            None,
            SHA_1,
            1234,
            1_762_700_000_000,
            vec![text_fehler("aa"), text("bb", SHA_4, "ok")],
        ),
        fassung(FAS_3, Some(FAS_2), SHA_4, 7, 1_762_700_200_000, vec![]),
    ];
    d.wahlen = vec![
        wahl(WAHL_1, FAS_2, &[FAS_1], &[], 4, G1),
        wahl(WAHL_2, FAS_3, &[FAS_2], &[WAHL_1], 6, G2),
    ];
    d.entfernt = vec![FAS_2.to_string()]; // nimmt FAS_2 aus dem Ergebnis
    d.papierkorb = reg(Some(1_762_800_000_000), 6, G2);
    d.extra = extra(r#"{"zukunft":{"a":2},"nur_b":true}"#);
    d
}

fn merge_ordner_a() -> Ordner {
    Ordner {
        id: ORD.to_string(),
        eltern: reg(None, 1, G1),
        name: reg("Verträge".to_string(), 2, G1),
        datenklasse: reg("erbt".to_string(), 1, G1),
        nur_geraet: reg(false, 1, G1),
        zielordner_ki: reg(false, 1, G1),
        papierkorb: reg(None, 1, G1),
        extra: extra(r#"{"zukunft":{"a":[1]}}"#),
    }
}

fn merge_ordner_b() -> Ordner {
    Ordner {
        id: ORD.to_string(),
        eltern: reg(Some(ORD_E.to_string()), 2, G2),
        // gleicher Rang (2, G1): der größere Wert gewinnt ("Verträge alt" ist länger als "Verträge")
        name: reg("Verträge alt".to_string(), 2, G1),
        datenklasse: reg("internal".to_string(), 3, G2),
        nur_geraet: reg(true, 1, G1), // gleicher Rang, größerer Wert (true > false)
        zielordner_ki: reg(true, 0, G2), // niedrigerer Rang verliert
        papierkorb: reg(Some(1_762_800_000_000), 2, G1),
        extra: extra(r#"{"zukunft":{"a":[2]},"neu":null}"#),
    }
}

/// Eine Datei in „falscher“ Reihenfolge: `hash_datei` sortiert nicht, `zusammenfuehren_datei` schon.
fn unsortiert() -> Datei {
    let mut d = datei_leer();
    let f1 = fassung(
        FAS_1,
        None,
        SHA_1,
        1234,
        1_762_700_000_000,
        vec![text("bb", SHA_4, "kein_text"), text("aa", SHA_3, "ok")],
    );
    let mut f2 = fassung(
        FAS_2,
        Some(FAS_1),
        SHA_2,
        u64::MAX,
        1_762_700_100_000,
        vec![text_fehler("ff")],
    );
    f2.geraet = G2.to_string();
    d.fassungen = vec![f2, f1];
    d.wahlen = vec![
        wahl(
            WAHL_2,
            FAS_2,
            &[FAS_1],
            &[WAHL_1],
            9_007_199_254_740_993,
            G2,
        ),
        wahl(WAHL_1, FAS_1, &[], &[], 5, G1),
    ];
    d.entfernt = vec![FAS_Z.to_string(), FAS_X.to_string(), FAS_Z.to_string()];
    d.blob_refs = vec!["veraltet".to_string()];
    d.extra = extra(r#"{"zusatz":{"zeta":null,"alpha":[1,2.5,"x"],"u":"ü"}}"#);
    d
}

/// Zwei Einträge mit gleicher Kennung und verschiedenem Inhalt: das Zusammenführen verdichtet sie (das
/// Größere nach der abgeleiteten Ordnung gewinnt), `hash_datei` ohne Zusammenführen nicht.
fn gleiche_kennung() -> Datei {
    let mut d = datei_leer();
    d.fassungen = vec![
        fassung(
            FAS_1,
            None,
            SHA_1,
            10,
            1_762_700_000_000,
            vec![text("aa", SHA_3, "ok"), text_fehler("aa")],
        ),
        fassung(FAS_1, None, SHA_1, 10, 1_762_700_900_000, vec![]),
    ];
    d.wahlen = vec![
        wahl(WAHL_1, FAS_1, &[], &[], 1, G1),
        wahl(WAHL_1, FAS_1, &[], &[], 2, G1),
        wahl(WAHL_1, FAS_1, &["x"], &[], 0, G1),
    ];
    d
}

fn zusammenfuehren() -> Value {
    let key = schluessel();
    let (a, b) = (merge_a(), merge_b());
    let m = zf::zusammenfuehren_datei(&a, &b);
    assert_eq!(m, zf::zusammenfuehren_datei(&b, &a), "kommutativ");
    assert_eq!(m, zf::zusammenfuehren_datei(&m, &m), "idempotent");
    assert!(
        m.fassungen.iter().all(|f| f.fas != FAS_2),
        "entfernt nimmt FAS_2 heraus"
    );

    let (oa, ob) = (merge_ordner_a(), merge_ordner_b());
    let om = zf::zusammenfuehren_ordner(&oa, &ob);
    assert_eq!(om, zf::zusammenfuehren_ordner(&ob, &oa), "kommutativ");

    let u = unsortiert();
    let u_sortiert = zf::zusammenfuehren_datei(&u, &u);
    assert_ne!(
        hash_datei(&u),
        hash_datei(&u_sortiert),
        "Beispiel ist wirklich unsortiert"
    );

    let g = gleiche_kennung();
    let g_verdichtet = zf::zusammenfuehren_datei(&g, &g);
    assert!(g_verdichtet.fassungen.len() == 1 && g_verdichtet.wahlen.len() == 1);

    json!({
        "datei": {
            "a": kanon_datei(&a), "b": kanon_datei(&b),
            "ergebnis": kanon_datei(&m), "ergebnis_hash": hash_datei(&m),
            "ergebnis_hub_refs": hub_refs(&key, &m),
        },
        "ordner": {
            "a": kanon_ordner(&oa), "b": kanon_ordner(&ob),
            "ergebnis": kanon_ordner(&om), "ergebnis_hash": hash_ordner(&om),
        },
        "unsortiert": {
            "serde_roh": serde_json::to_string(&u).unwrap(),
            "treu_kanonisch": kanon_datei(&u), "treu_hash": hash_datei(&u),
            "sortiert_kanonisch": kanon_datei(&u_sortiert), "sortiert_hash": hash_datei(&u_sortiert),
            "hub_refs": hub_refs(&key, &u),
        },
        "gleiche_kennung": {
            "serde_roh": serde_json::to_string(&g).unwrap(),
            "treu_kanonisch": kanon_datei(&g), "treu_hash": hash_datei(&g),
            "verdichtet_kanonisch": kanon_datei(&g_verdichtet), "verdichtet_hash": hash_datei(&g_verdichtet),
        },
    })
}

/// Alle Zusatzvektoren; Quelle des Schlüssels `zusatz` in `ablage-testvektoren.json`.
fn alle() -> Value {
    json!({
        "hinweis": "Zusatzvektoren der Dokumentenablage (D5). Erzeugt von sync::ablage::vektoren_zusatz::json_passt_zum_code. Neu schreiben nur mit LVA_VEKTOREN_SCHREIBEN=1 (z. B. LVA_VEKTOREN_SCHREIBEN=1 cargo test --lib sync::ablage::vektoren_zusatz::json_passt_zum_code); ohne die Variable prüft der Test nur. Nicht von Hand ändern. Gruppen: escapes, zahlen, umlaut_schluessel, serde_lesefaelle, id_gueltig, tiefe_127, hub_hex, grabstein_benutzer, zusammenfuehren.",
        "user_id": USER,
        "escapes": escapes(),
        "zahlen": zahlen(),
        "umlaut_schluessel": umlaut_schluessel(),
        "serde_lesefaelle": serde_lesefaelle(),
        "id_gueltig": id_gueltig_faelle(),
        "tiefe_127": tiefe_127(),
        "hub_hex": hub_hex(),
        "grabstein_benutzer": grabstein_benutzer(),
        "zusammenfuehren": zusammenfuehren(),
    })
}

/// Der Schlüssel `zusatz` der Datei enthält genau das, was der Code heute berechnet.
#[test]
fn json_passt_zum_code() {
    let soll = json_abgleichen(Some("zusatz"), &alle());
    // Der Grundsatz bleibt unberührt: `zusatz` kommt nur zusätzlich vor.
    assert!(soll.get("aad").is_some() && soll.get("nutzlast").is_some());
}

#[test]
fn tiefe_ist_127() {
    let t = tiefe_127();
    assert_eq!(t["max_tiefe_array"], 127);
    assert_eq!(t["max_tiefe_objekt"], 127);
}

/// Die Gleitkomma-Schreibweise hängt an der serde_json-Version (1.0.149: `1e+21`, ältere: `1e21`); fällt
/// dieser Test, ändert sich der Hash von Zusatzfeldern mit Gleitkommazahlen und Swift muss mitziehen.
#[test]
fn gleitkomma_schreibweise() {
    let z = zahlen();
    let aus = |ein: &str| -> String {
        z["faelle"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["ein"] == ein)
            .and_then(|f| f["aus"].as_str())
            .unwrap_or_default()
            .to_string()
    };
    assert_eq!(aus("1e21"), "1e+21");
    assert_eq!(aus("1e-7"), "1e-7");
    assert_eq!(aus("100.0"), "100.0");
    assert_eq!(aus("-0"), "-0.0");
    assert_eq!(aus("1e999"), "");
}
