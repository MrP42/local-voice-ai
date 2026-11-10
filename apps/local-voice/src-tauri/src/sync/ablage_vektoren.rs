//! Feste Testvektoren der Ablage für den iOS-Port (D5, Epic #395). Wie `blobs::tests::testvektor_fuer_swift`:
//! feste Eingaben, festes erwartetes Ergebnis; Swift (`VoiceCore`) rechnet dieselben Eingaben nach und
//! vergleicht mit denselben Konstanten. Maschinenlesbar stehen alle Vektoren in
//! `docs/superpowers/plans/ablage-testvektoren.json` (`json_passt_zum_code` prüft, dass die Datei zum Code
//! passt; mit `LVA_VEKTOREN_SCHREIBEN=1` schreibt derselbe Test sie neu).
//!
//! Alle Schlüssel, Kennungen und Namen sind synthetisch (Schlüssel `00 01 … 1f`, Benutzer 7).

use super::*;
use crate::ablage::extrakt::{self, Abschnitt, Textfassung};
use crate::ablage::zusammenfuehren::{Fassung, Text, Wahl};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::json;
use std::collections::BTreeMap;

const USER: i64 = 7;
const ORD: &str = "ord_01HZX3K9M2N4P5Q6R7S8T9V0WX";
const DAT: &str = "dat_01HZX3K9M2N4P5Q6R7S8T9V0WY";
const SHA_ORIG_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SHA_ORIG_2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const SHA_TEXT_1: &str = "3333333333333333333333333333333333333333333333333333333333333333";

fn schluessel() -> [u8; KEY_LEN] {
    let mut k = [0u8; KEY_LEN];
    for (i, b) in k.iter_mut().enumerate() {
        *b = i as u8;
    }
    k
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn reg<T>(wert: T, zaehler: u64, geraet: &str) -> Register<T> {
    Register::neu(wert, zaehler, geraet)
}

fn ordner() -> Ordner {
    let mut extra = BTreeMap::new();
    extra.insert(
        "zukunft".to_string(),
        json!({"z": null, "a": [3, "ü", null], "m": true}),
    );
    Ordner {
        id: ORD.to_string(),
        eltern: reg(None, 1, "g_01HZX3K9M2N4P5Q6R7S8T9V0W1"),
        name: reg(
            "Verträge – Übersicht".to_string(),
            2,
            "g_01HZX3K9M2N4P5Q6R7S8T9V0W1",
        ),
        datenklasse: reg("erbt".to_string(), 1, "g_01HZX3K9M2N4P5Q6R7S8T9V0W1"),
        nur_geraet: reg(false, 1, "g_01HZX3K9M2N4P5Q6R7S8T9V0W1"),
        zielordner_ki: reg(false, 1, "g_01HZX3K9M2N4P5Q6R7S8T9V0W1"),
        papierkorb: reg(None, 1, "g_01HZX3K9M2N4P5Q6R7S8T9V0W1"),
        extra,
    }
}

fn datei() -> Datei {
    let g = "g_01HZX3K9M2N4P5Q6R7S8T9V0W1";
    let text = |rev: &str, blob: &str, stand: &str| Text {
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
    };
    let mut fehler = text("ff", "", "fehler");
    fehler.code = Some("extrakt.zeit".to_string());
    fehler.versuche = 2;
    fehler.versuche_zeit = 1;
    let mut extra = BTreeMap::new();
    extra.insert("zukunft".to_string(), json!({"b": null, "a": 1}));
    Datei {
        id: DAT.to_string(),
        ordner: reg(ORD.to_string(), 3, g),
        name: reg("Mietvertrag Köln.txt".to_string(), 2, g),
        klasse_min: reg("public".to_string(), 1, g),
        nur_geraet_min: false,
        fassungen: vec![
            Fassung {
                fas: "fas_01HZX3K9M2N4P5Q6R7S8T9V0W2".to_string(),
                vorgaenger: None,
                blob: SHA_ORIG_1.to_string(),
                size: 1234,
                original_sha256: SHA_ORIG_1.to_string(),
                typ: "txt".to_string(),
                geraet: g.to_string(),
                zeit: 1_762_700_000_000,
                texte: vec![text("aa", SHA_TEXT_1, "ok"), fehler],
            },
            Fassung {
                fas: "fas_01HZX3K9M2N4P5Q6R7S8T9V0W3".to_string(),
                vorgaenger: Some("fas_01HZX3K9M2N4P5Q6R7S8T9V0W2".to_string()),
                blob: SHA_ORIG_2.to_string(),
                size: 99,
                original_sha256: SHA_ORIG_2.to_string(),
                typ: "txt".to_string(),
                geraet: g.to_string(),
                zeit: 1_762_700_100_000,
                texte: vec![],
            },
        ],
        wahlen: vec![Wahl {
            id: "wahl_01HZX3K9M2N4P5Q6R7S8T9V0W4".to_string(),
            gewaehlt: "fas_01HZX3K9M2N4P5Q6R7S8T9V0W3".to_string(),
            gegen: vec!["fas_01HZX3K9M2N4P5Q6R7S8T9V0W2".to_string()],
            ersetzt: vec![],
            zaehler: 4,
            geraet: g.to_string(),
        }],
        entfernt: vec![],
        blob_refs: vec![],
        papierkorb: reg(None, 1, g),
        extra,
    }
}

fn textfassung() -> Textfassung {
    let rev = extrakt::textrev(SHA_ORIG_1, "lva-text", "1", 1);
    Textfassung {
        marken_version: 1,
        textrev: rev,
        original_sha256: SHA_ORIG_1.to_string(),
        extraktor: "lva-text".to_string(),
        extraktor_version: "1".to_string(),
        abschnitte: vec![
            Abschnitt {
                anker: "zeile=1".to_string(),
                pfad: vec![],
                text: "Die Miete beträgt 850 Euro.".to_string(),
            },
            Abschnitt {
                anker: "absatz=2".to_string(),
                pfad: vec!["§ 4 Laufzeit".to_string()],
                text: "Kündigung \"zum Quartal\" – ohne Frist.".to_string(),
            },
        ],
    }
}

/// Die Fälle der AAD: (Name, Sammlung, Objekt, Hub-Kennungen, Grabstein).
fn aad_faelle(key: &[u8; KEY_LEN]) -> Vec<(&'static str, &'static str, String, Vec<String>, bool)> {
    let refs = hub_refs(key, &datei());
    vec![
        ("ordner", "ablage_ordner", ORD.to_string(), vec![], false),
        (
            "ordner_grabstein",
            "ablage_ordner",
            ORD.to_string(),
            vec![],
            true,
        ),
        (
            "datei_ohne_refs",
            "ablage_datei",
            DAT.to_string(),
            vec![],
            false,
        ),
        (
            "datei_mit_refs",
            "ablage_datei",
            DAT.to_string(),
            refs.clone(),
            false,
        ),
        (
            "datei_ein_ref",
            "ablage_datei",
            DAT.to_string(),
            vec![refs[0].clone()],
            false,
        ),
        (
            "datei_grabstein",
            "ablage_datei",
            DAT.to_string(),
            vec![],
            true,
        ),
    ]
}

/// Alle Vektoren als JSON; Quelle der Datei `ablage-testvektoren.json`.
fn alle() -> Value {
    let key = schluessel();
    let kid = crypto::key_id(&key);
    let aad: Vec<Value> = aad_faelle(&key)
        .into_iter()
        .map(|(name, coll, id, refs, del)| {
            let a = crypto::aad_ablage(USER, coll, &id, &refs, &kid, del);
            json!({"name": name, "user_id": USER, "collection": coll, "object_id": id,
                   "blob_refs": refs, "key_id": kid, "deleted": del, "aad_hex": hex(&a),
                   "aad_utf8": String::from_utf8(a).unwrap_or_default()})
        })
        .collect();
    let grab: Vec<Value> = [("ablage_ordner", ORD), ("ablage_datei", DAT)]
        .into_iter()
        .map(|(coll, id)| {
            let klar = grab_klartext(coll, id);
            let a = crypto::aad_ablage(USER, coll, id, &[], &kid, true);
            let sealed = grab_nutzlast(&key, USER, coll, id).unwrap_or_default();
            let roh = B64.decode(&sealed).unwrap_or_default();
            json!({"collection": coll, "object_id": id, "klartext": klar,
                   "aad_hex": hex(&a), "nonce_hex": hex(&roh[..24]),
                   "payload_base64": sealed, "payload_hex": hex(&roh)})
        })
        .collect();
    let o = ordner();
    let d = datei();
    let fassung = textfassung();
    let bytes = serde_json::to_vec(&fassung).unwrap_or_default();
    let rev = |p: &str, e: &str, v: &str, m: u32| {
        json!({"original_sha256": p, "extraktor": e, "extraktor_version": v,
               "marken_version": m, "textrev": extrakt::textrev(p, e, v, m)})
    };
    let hub: Vec<Value> = [SHA_ORIG_1, SHA_ORIG_2, SHA_TEXT_1]
        .iter()
        .map(|s| json!({"sha256": s, "hub_kennung": hub_blob(&key, s)}))
        .collect();
    json!({
        "version": 1,
        "hinweis": "Synthetische Testvektoren der Dokumentenablage (D5). Erzeugt von sync::ablage::vektoren::json_passt_zum_code (LVA_VEKTOREN_SCHREIBEN=1). Nicht von Hand ändern.",
        "schluessel_hex": hex(&key),
        "user_id": USER,
        "key_id": kid,
        "aad": aad,
        "grabstein": grab,
        "textrev": [
            rev(SHA_ORIG_1, "lva-text", "1", 1),
            rev(SHA_ORIG_1, "lva-text", "1", 2),
            rev(SHA_ORIG_2, "pdf-extract", "0.12.0+lva1", 1),
        ],
        "textfassung": {
            "datei": DAT,
            "json_utf8": String::from_utf8(bytes.clone()).unwrap_or_default(),
            "sha256": blobs::sha256_hex(&bytes),
            "hub_kennung": hub_blob(&key, &blobs::sha256_hex(&bytes)),
        },
        "standard_id": standard_id(&key),
        "hub_kennungen": hub,
        "nutzlast": {
            "ordner": {"kanonisch": collect::canonical(&wert_ordner(&o)), "hash": hash_ordner(&o)},
            "datei": {"kanonisch": collect::canonical(&wert_datei(&d)), "hash": hash_datei(&d),
                      "blob_refs_inhalt": zf::blob_refs_aus(&d.fassungen),
                      "blob_refs_hub": hub_refs(&key, &d)},
        },
    })
}

// BEGIN KONSTANTEN
/// Schlüssel 00..1f: erste 4 Byte von SHA-256(Schlüssel), hex.
const KEY_ID: &str = "630dcd29";
/// (Name, AAD als Hex) in der Reihenfolge von `aad_faelle`; Benutzer 7.
const AAD_HEX: &[(&str, &str)] = &[
    ("ordner", "371f61626c6167655f6f72646e65721f6f72645f3031485a58334b394d324e3450355136523753385439563057581f626c6f625f726566731f1f6b65795f69641f36333064636432391f64656c657465641f30"),
    ("ordner_grabstein", "371f61626c6167655f6f72646e65721f6f72645f3031485a58334b394d324e3450355136523753385439563057581f626c6f625f726566731f1f6b65795f69641f36333064636432391f64656c657465641f31"),
    ("datei_ohne_refs", "371f61626c6167655f64617465691f6461745f3031485a58334b394d324e3450355136523753385439563057591f626c6f625f726566731f1f6b65795f69641f36333064636432391f64656c657465641f30"),
    ("datei_mit_refs", "371f61626c6167655f64617465691f6461745f3031485a58334b394d324e3450355136523753385439563057591f626c6f625f726566731f326336306137326434343763316662363338303831363063383365616432343231323766393431653137636664306264616233373437376534306330666536612c383837383236353132316162393830326539623035303937653661653132346366326433346666623530376662303331346632363037316264396130323862362c623065363030323935386538666437356137396266626537343830363130333537633962373630663337616531613563616162343931323465643730346534341f6b65795f69641f36333064636432391f64656c657465641f30"),
    ("datei_ein_ref", "371f61626c6167655f64617465691f6461745f3031485a58334b394d324e3450355136523753385439563057591f626c6f625f726566731f326336306137326434343763316662363338303831363063383365616432343231323766393431653137636664306264616233373437376534306330666536611f6b65795f69641f36333064636432391f64656c657465641f30"),
    ("datei_grabstein", "371f61626c6167655f64617465691f6461745f3031485a58334b394d324e3450355136523753385439563057591f626c6f625f726566731f1f6b65795f69641f36333064636432391f64656c657465641f31")
];
/// (SHA-256 des Klartexts, Hub-Kennung) zum Schlüssel 00..1f.
const HUB_KENNUNGEN: &[(&str, &str)] = &[
    (
        "1111111111111111111111111111111111111111111111111111111111111111",
        "8878265121ab9802e9b05097e6ae124cf2d34ffb507fb0314f26071bd9a028b6",
    ),
    (
        "2222222222222222222222222222222222222222222222222222222222222222",
        "2c60a72d447c1fb63808160c83ead242127f941e17cfd0bdab37477e40c0fe6a",
    ),
    (
        "3333333333333333333333333333333333333333333333333333333333333333",
        "b0e6002958e8fd75a79bfbe7480610357c9b760f37ae1a5caab49124ed704e44",
    ),
];
/// (Sammlung, Objekt, Klartext, AAD hex, versiegelte Nutzlast base64).
const GRABSTEINE: &[(&str, &str, &str, &str, &str)] = &[
    ("ablage_ordner", "ord_01HZX3K9M2N4P5Q6R7S8T9V0WX", r##"{"collection":"ablage_ordner","deleted":true,"id":"ord_01HZX3K9M2N4P5Q6R7S8T9V0WX"}"##, "371f61626c6167655f6f72646e65721f6f72645f3031485a58334b394d324e3450355136523753385439563057581f626c6f625f726566731f1f6b65795f69641f36333064636432391f64656c657465641f31", "2IM87PdfYxvG9ULQAyf1Ig73ZzHV//wyosG/RodfNLlhluFsmC0o5YzSM/kMcPqHO2QApslWxfnlCl77t0EZ4caL+WZRIzJxvH60ISHhmD2R44lpvvuEIbLw+bTOM12ATtKWKmfYHdvkI1GEZfBAK23ytA9j69dJpeW9"),
    ("ablage_datei", "dat_01HZX3K9M2N4P5Q6R7S8T9V0WY", r##"{"collection":"ablage_datei","deleted":true,"id":"dat_01HZX3K9M2N4P5Q6R7S8T9V0WY"}"##, "371f61626c6167655f64617465691f6461745f3031485a58334b394d324e3450355136523753385439563057591f626c6f625f726566731f1f6b65795f69641f36333064636432391f64656c657465641f31", "0U/+M2Tg5maW6JMKX43gZwIxJ7d9BWD0xv4d3nhYz9L2IIVKlUN2MDPvmbVHGw2RigXRiuWcF4Tw2G28WkesV4O2CEs/0RmhFd1L5vgBwQZTV9us6n2cXihAagaTu31TTMbd2zGEnzfS8DB1L87VYk8lvI8xaopeTPM=")
];
const STANDARD_ID: &str = "ord_4TCSNDVY1EXTCJA0K1B37445HE";
const ORDNER_KANONISCH: &str = r##"{"datenklasse":{"geraet":"g_01HZX3K9M2N4P5Q6R7S8T9V0W1","wert":"erbt","zaehler":1},"eltern":{"geraet":"g_01HZX3K9M2N4P5Q6R7S8T9V0W1","wert":null,"zaehler":1},"id":"ord_01HZX3K9M2N4P5Q6R7S8T9V0WX","name":{"geraet":"g_01HZX3K9M2N4P5Q6R7S8T9V0W1","wert":"Verträge – Übersicht","zaehler":2},"nur_geraet":{"geraet":"g_01HZX3K9M2N4P5Q6R7S8T9V0W1","wert":false,"zaehler":1},"papierkorb":{"geraet":"g_01HZX3K9M2N4P5Q6R7S8T9V0W1","wert":null,"zaehler":1},"zielordner_ki":{"geraet":"g_01HZX3K9M2N4P5Q6R7S8T9V0W1","wert":false,"zaehler":1},"zukunft":{"a":[3,"ü",null],"m":true,"z":null}}"##;
const ORDNER_HASH: &str = "3b1a06b073e14c54eced12dbb37f77367fe15655a4235d9fa4b1422e92c30bb8";
const DATEI_KANONISCH: &str = r##"{"blob_refs":["1111111111111111111111111111111111111111111111111111111111111111","2222222222222222222222222222222222222222222222222222222222222222","3333333333333333333333333333333333333333333333333333333333333333"],"entfernt":[],"fassungen":[{"blob":"1111111111111111111111111111111111111111111111111111111111111111","fas":"fas_01HZX3K9M2N4P5Q6R7S8T9V0W2","geraet":"g_01HZX3K9M2N4P5Q6R7S8T9V0W1","original_sha256":"1111111111111111111111111111111111111111111111111111111111111111","size":1234,"texte":[{"blob":"3333333333333333333333333333333333333333333333333333333333333333","extraktor":"lva-text","extraktor_version":"1","marken_version":1,"size":420,"stand":"ok","textrev":"aa"},{"blob":"","code":"extrakt.zeit","extraktor":"lva-text","extraktor_version":"1","marken_version":1,"size":420,"stand":"fehler","textrev":"ff","versuche":2,"versuche_zeit":1}],"typ":"txt","vorgaenger":null,"zeit":1762700000000},{"blob":"2222222222222222222222222222222222222222222222222222222222222222","fas":"fas_01HZX3K9M2N4P5Q6R7S8T9V0W3","geraet":"g_01HZX3K9M2N4P5Q6R7S8T9V0W1","original_sha256":"2222222222222222222222222222222222222222222222222222222222222222","size":99,"texte":[],"typ":"txt","vorgaenger":"fas_01HZX3K9M2N4P5Q6R7S8T9V0W2","zeit":1762700100000}],"id":"dat_01HZX3K9M2N4P5Q6R7S8T9V0WY","klasse_min":{"geraet":"g_01HZX3K9M2N4P5Q6R7S8T9V0W1","wert":"public","zaehler":1},"name":{"geraet":"g_01HZX3K9M2N4P5Q6R7S8T9V0W1","wert":"Mietvertrag Köln.txt","zaehler":2},"nur_geraet_min":false,"ordner":{"geraet":"g_01HZX3K9M2N4P5Q6R7S8T9V0W1","wert":"ord_01HZX3K9M2N4P5Q6R7S8T9V0WX","zaehler":3},"papierkorb":{"geraet":"g_01HZX3K9M2N4P5Q6R7S8T9V0W1","wert":null,"zaehler":1},"wahlen":[{"ersetzt":[],"gegen":["fas_01HZX3K9M2N4P5Q6R7S8T9V0W2"],"geraet":"g_01HZX3K9M2N4P5Q6R7S8T9V0W1","gewaehlt":"fas_01HZX3K9M2N4P5Q6R7S8T9V0W3","id":"wahl_01HZX3K9M2N4P5Q6R7S8T9V0W4","zaehler":4}],"zukunft":{"a":1,"b":null}}"##;
const DATEI_HASH: &str = "e5f2c4f489ee48457ba50758ae20afb4dea5cfa1daf215623ade52c781bdfe0d";
const TEXTFASSUNG_JSON: &str = r##"{"marken_version":1,"textrev":"22ce1b7c5b6b11423d01124bd33bb0e4a56b6178cc726f7bc050a8bed96fec92","original_sha256":"1111111111111111111111111111111111111111111111111111111111111111","extraktor":"lva-text","extraktor_version":"1","abschnitte":[{"anker":"zeile=1","text":"Die Miete beträgt 850 Euro."},{"anker":"absatz=2","pfad":["§ 4 Laufzeit"],"text":"Kündigung \"zum Quartal\" – ohne Frist."}]}"##;
const TEXTFASSUNG_SHA256: &str = "c0b677fae8e4a2e4c3ba7ca24bbb96659d84457a573552771f819835437aa344";
// END KONSTANTEN

fn g(v: &Value, pfad: &[&str]) -> String {
    pfad.iter()
        .fold(v, |a, k| &a[*k])
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// AAD: `"7" 1F "ablage_datei" 1F <id> 1F "blob_refs" 1F <refs, mit Komma> 1F "key_id" 1F <key_id> 1F
/// "deleted" 1F "0"|"1"`. Swift: dieselben Bytes bauen (`SyncCrypto.aad` + Anhang) und als Hex vergleichen.
#[test]
fn aad_bytes() {
    let key = schluessel();
    let kid = crypto::key_id(&key);
    assert_eq!(kid, KEY_ID);
    for (name, coll, id, refs, del) in aad_faelle(&key) {
        let a = crypto::aad_ablage(USER, coll, &id, &refs, &kid, del);
        let erwartet = AAD_HEX.iter().find(|(n, _)| *n == name).expect(name).1;
        assert_eq!(hex(&a), erwartet, "{name}");
    }
    assert_eq!(AAD_HEX.len(), aad_faelle(&key).len());
}

/// Hub-Kennung: `hex(SHA256(Schlüssel ‖ "lv-blob-v1" ‖ Roh-SHA-256))`. Swift: `LibrarySyncFormat.blobId`.
#[test]
fn hub_kennungen() {
    let key = schluessel();
    for (sha, erwartet) in HUB_KENNUNGEN {
        assert_eq!(hub_blob(&key, sha).as_deref(), Some(*erwartet));
    }
    assert_eq!(hub_blob(&key, "zz"), None);
}

/// Grabstein: Klartext `{"collection":…,"deleted":true,"id":…}`, AAD mit `deleted=1` und leeren Refs,
/// Nonce = SHA-256("lv-ablage-siegel-nonce" 1F ‖ Schlüssel ‖ u64-BE(len aad) ‖ aad ‖ Klartext)[0..24],
/// XChaCha20-Poly1305, Ausgabe `base64(Nonce ‖ Chiffrat ‖ Tag)`. Swift rechnet mit demselben Nonce-Hash
/// und vergleicht Base64 (Chiffrat ist deterministisch).
#[test]
fn grabstein_siegel() {
    let key = schluessel();
    let kid = crypto::key_id(&key);
    for (coll, id, klar, aad_hex, payload) in GRABSTEINE {
        assert_eq!(grab_klartext(coll, id), *klar);
        let a = crypto::aad_ablage(USER, coll, id, &[], &kid, true);
        assert_eq!(hex(&a), *aad_hex);
        assert_eq!(grab_nutzlast(&key, USER, coll, id).unwrap(), *payload);
        assert!(grab_echt(&key, USER, coll, id, &kid, payload));
        // Nonce-Regel unabhängig nachgerechnet.
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"lv-ablage-siegel-nonce\x1f");
        h.update(key);
        h.update((a.len() as u64).to_be_bytes());
        h.update(&a);
        h.update(klar.as_bytes());
        let n = h.finalize();
        let roh = B64.decode(payload).unwrap();
        assert_eq!(&roh[..24], &n[..24]);
    }
    // Falsche Sammlung oder falsche Kennung öffnet nicht.
    let (coll, id, _, _, payload) = GRABSTEINE[0];
    assert!(!grab_echt(&key, USER, "ablage_datei", id, &kid, payload));
    assert!(!grab_echt(&key, USER, coll, DAT, &kid, payload));
}

/// Standardordner: `"ord_" + ULID(SHA256(Schlüssel ‖ "lv-ablage-allgemein")[0..16])`, 26 Zeichen
/// Crockford-Base32 (Großbuchstaben, 128 Bit, höchstwertige 5-Bit-Gruppe zuerst mit zwei Füllbits oben).
#[test]
fn standard_ordner_kennung() {
    let key = schluessel();
    let id = standard_id(&key);
    assert_eq!(id, STANDARD_ID);
    assert!(crate::ablage::id_gueltig(
        crate::ablage::PRAEFIX_ORDNER,
        &id
    ));
}

/// Kanonische Nutzlast: Schlüssel rekursiv nach Bytes sortiert, kompaktes JSON, `null` bleibt (auch in
/// unbekannten Feldern), Nicht-ASCII bleibt Klartext, Listen in gegebener Reihenfolge; Hash =
/// SHA-256(UTF-8-Bytes). Bei der Datei sind `blob_refs` die Inhalts-Hashes, neu berechnet (Eingabe leer).
/// Swift darf `null` nicht streichen (anders als `LibrarySyncFormat.canonicalHash`).
#[test]
fn kanonische_nutzlast_und_hash() {
    let o = ordner();
    let k = collect::canonical(&wert_ordner(&o));
    assert_eq!(k, ORDNER_KANONISCH);
    assert_eq!(hash_ordner(&o), ORDNER_HASH);
    assert_eq!(blobs::sha256_hex(k.as_bytes()), ORDNER_HASH);
    assert!(k.contains("\"wert\":null"));

    let d = datei();
    let k = collect::canonical(&wert_datei(&d));
    assert_eq!(k, DATEI_KANONISCH);
    assert_eq!(hash_datei(&d), DATEI_HASH);
    assert_eq!(blobs::sha256_hex(k.as_bytes()), DATEI_HASH);
    assert!(k.contains("\"vorgaenger\":null"));
    // Die gespeicherte Eingabe `blob_refs: []` ändert nichts: sie wird aus den Fassungen abgeleitet.
    let mut d2 = d.clone();
    d2.blob_refs = vec!["x".to_string()];
    assert_eq!(hash_datei(&d2), DATEI_HASH);
}

/// Pfad der gemeinsamen Vektordatei.
pub(super) fn json_pfad() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../docs/superpowers/plans/ablage-testvektoren.json"
    )
}

static JSON_SPERRE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Gleicht einen Teil der gemeinsamen Vektordatei mit dem ab, was der Code heute berechnet, und liefert
/// den Dateiinhalt. `teil`: `None` = alle Schlüssel außer `zusatz` (dieses Modul), `Some(schluessel)` = genau
/// dieser Schlüssel (`ablage_vektoren_zusatz`). Nur bei `LVA_VEKTOREN_SCHREIBEN=1` wird der Teil in die Datei
/// geschrieben (der Wert zählt, nicht das Vorhandensein); die übrigen Teile bleiben unberührt. Beide Tests
/// laufen parallel, daher Lesen und Schreiben unter einer gemeinsamen Sperre.
pub(super) fn json_abgleichen(teil: Option<&str>, ist: &Value) -> Value {
    let _g = JSON_SPERRE.lock().unwrap_or_else(|e| e.into_inner());
    let pfad = json_pfad();
    let lesen =
        || -> Option<Value> { serde_json::from_str(&std::fs::read_to_string(pfad).ok()?).ok() };
    if std::env::var("LVA_VEKTOREN_SCHREIBEN").as_deref() == Ok("1") {
        let mut neu = lesen().unwrap_or_else(|| json!({}));
        let map = neu.as_object_mut().expect("Vektordatei ist kein Objekt");
        match teil {
            None => {
                let zusatz = map.remove("zusatz");
                *map = ist.as_object().expect("Vektoren sind kein Objekt").clone();
                if let Some(z) = zusatz {
                    map.insert("zusatz".to_string(), z);
                }
            }
            Some(k) => {
                map.insert(k.to_string(), ist.clone());
            }
        }
        std::fs::write(pfad, serde_json::to_string_pretty(&neu).unwrap() + "\n").unwrap();
    }
    let soll = lesen().expect("ablage-testvektoren.json fehlt oder ist kein JSON");
    match teil {
        None => {
            let mut ohne = soll.clone();
            ohne.as_object_mut().unwrap().remove("zusatz");
            assert_eq!(ohne, *ist, "ablage-testvektoren.json weicht vom Code ab");
        }
        Some(k) => assert_eq!(
            soll.get(k),
            Some(ist),
            "ablage-testvektoren.json (`{k}`) weicht vom Code ab; neu schreiben mit LVA_VEKTOREN_SCHREIBEN=1"
        ),
    }
    soll
}

/// Die JSON-Datei enthält genau das, was der Code heute berechnet (ohne den Schlüssel `zusatz`, den
/// `ablage_vektoren_zusatz` prüft).
#[test]
fn json_passt_zum_code() {
    let soll = json_abgleichen(None, &alle());
    // Und die Konstanten dieses Moduls stimmen mit der Datei überein.
    assert_eq!(g(&soll, &["key_id"]), KEY_ID);
    assert_eq!(g(&soll, &["standard_id"]), STANDARD_ID);
    assert_eq!(g(&soll, &["nutzlast", "datei", "hash"]), DATEI_HASH);
    assert_eq!(
        g(&soll, &["nutzlast", "ordner", "kanonisch"]),
        ORDNER_KANONISCH
    );
    assert_eq!(g(&soll, &["textfassung", "sha256"]), TEXTFASSUNG_SHA256);
}

/// Textfassung als Bytes (`serde_json::to_vec`, Feldreihenfolge der Struktur, `pfad` fehlt wenn leer),
/// SHA-256 der Bytes = `Text.blob`. Die `textrev`-Vektoren stehen in `ablage::textfassung::vektoren`.
#[test]
fn textfassung_bytes() {
    let bytes = serde_json::to_vec(&textfassung()).unwrap();
    assert_eq!(String::from_utf8(bytes.clone()).unwrap(), TEXTFASSUNG_JSON);
    assert_eq!(blobs::sha256_hex(&bytes), TEXTFASSUNG_SHA256);
}
