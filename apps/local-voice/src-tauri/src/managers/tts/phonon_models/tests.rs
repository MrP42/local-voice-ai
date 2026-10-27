//! Tests zu `phonon_models`: Katalog, Ablage, Entpacken, Token, Download gegen einen Fake-Server.
//! Nichts hier beruehrt echte App-Daten oder das Netz (nur Loopback).

use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write as _;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const TOKEN: &str = "hf_GeheimesPhononToken_9876543210";

fn sha_hex(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

fn git_blob_of(bytes: &[u8]) -> String {
    let mut sha = Sha1::new();
    sha.update(format!("blob {}\0", bytes.len()).as_bytes());
    sha.update(bytes);
    sha.finish()
}

// ---- Katalog und Pfade ------------------------------------------------------------------------

#[test]
fn ids_und_dateinamen_im_katalog_sind_sichere_pfadbausteine() {
    for purpose in [
        Purpose::PhononRuntime,
        Purpose::PhononModel,
        Purpose::PhononVoice,
    ] {
        for e in catalog::tts_entries(purpose) {
            assert!(is_safe_component(&e.id), "{}", e.id);
            for f in &e.files {
                assert!(is_safe_component(&f.filename), "{}: {}", e.id, f.filename);
            }
            assert!(ensure_safe_entry(&e).is_ok(), "{}", e.id);
        }
    }
    for bad in [
        "",
        ".",
        "..",
        "../x",
        "a/b",
        "a\\b",
        ".versteckt",
        "a:b",
        "ä",
        "x y",
    ] {
        assert!(!is_safe_component(bad), "{bad:?}");
    }
}

#[test]
fn die_pfade_liegen_unter_tts_phonon() {
    let dirs = PhononDirs::new(Path::new("/data"));
    assert_eq!(
        dirs.runtime_dir("windows-x64"),
        Path::new("/data/tts/phonon/windows-x64")
    );
    assert_eq!(
        dirs.model_dir("phonon-40m-de"),
        Path::new("/data/tts/phonon/models/phonon-40m-de")
    );
    let voice = voice_entries()
        .into_iter()
        .find(|e| e.id == "phonon-voice-de-nele")
        .unwrap();
    assert_eq!(
        voice_file(&dirs, &voice).unwrap(),
        Path::new("/data/tts/phonon/models/phonon-40m-de/voices/Nele.safetensors")
    );
    assert_eq!(exe_name("windows-x64"), "ptts.exe");
    assert_eq!(exe_name("macos-aarch64"), "ptts");
    assert_eq!(exe_name("linux-x64"), "ptts");
}

#[test]
fn nur_phonon_kennungen_gehoeren_zu_phonon() {
    assert!(is_phonon_id("phonon-runtime"));
    assert!(is_phonon_id("phonon-40m-de"));
    assert!(is_phonon_id("phonon-31m-en"));
    assert!(is_phonon_id("phonon-voice-de-nele"));
    for other in [
        "piper-runtime",
        "de_DE-thorsten-high",
        "",
        "../phonon-40m-de",
        "phonon-40m-de/../x",
    ] {
        assert!(!is_phonon_id(other), "{other:?}");
    }
}

// ---- Liste --------------------------------------------------------------------------------------

#[test]
fn die_liste_nennt_programm_und_modelle_ohne_einzelne_stimmen_und_nichts_ist_installiert() {
    let tmp = TempDir::new().unwrap();
    let dirs = PhononDirs::new(tmp.path());
    let rows = build_downloads(&dirs, Some("windows-x64"), |_| false);
    assert_eq!(
        rows.len(),
        1 + 2,
        "Programm und zwei Modelle, keine Stimmenzeilen"
    );
    assert!(rows.iter().all(|r| r.kind != TtsDownloadKind::Voice));
    assert!(rows.iter().all(|r| r.engine == "phonon"));
    assert!(rows
        .iter()
        .all(|r| !r.is_downloaded && !r.is_usable && !r.is_downloading));

    let rt = &rows[0];
    assert_eq!(
        (rt.id.as_str(), rt.kind),
        (RUNTIME_ID, TtsDownloadKind::Runtime)
    );
    assert_eq!(rt.size_bytes, 7_766_316);
    assert_eq!(
        rt.sha256.as_deref(),
        Some("699f34526e66523a2f59c76fcf820f28fb11c581ea135cfb1e918c8d786b8a07")
    );
    assert_eq!(rt.license.as_deref(), Some("MIT OR Apache-2.0"));
    assert!(!rt.license_non_commercial && !rt.gated);
    assert!(rt.requires.is_none());

    let de = rows.iter().find(|r| r.id == "phonon-40m-de").unwrap();
    assert_eq!(de.kind, TtsDownloadKind::Model);
    assert!(de.gated && de.license_non_commercial);
    assert_eq!(de.license.as_deref(), Some("CC-BY-NC-4.0"));
    assert_eq!(de.language.as_deref(), Some("de"));
    assert_eq!(de.requires.as_deref(), Some(RUNTIME_ID));
    assert!(
        de.sha256.is_none(),
        "mehrere Dateien: kein gemeinsamer SHA-256"
    );
    assert!(de
        .source
        .as_deref()
        .unwrap()
        .starts_with("hf://gradium/phonon-40m-de@79cfc7b4"));
    let en = rows.iter().find(|r| r.id == "phonon-31m-en").unwrap();
    assert!(!en.gated && !en.license_non_commercial);

    // Die Groesse eines Modells ist die ganze Installation: Modell plus Stimmen.
    let model_bytes = |id: &str| files_bytes(&model_entry(id).unwrap());
    assert_eq!(de.size_bytes, 75_158_617 + 13 * 256_136);
    assert_eq!(model_bytes("phonon-40m-de"), 75_158_617);
    assert_eq!(en.size_bytes, 65_116_929 + 37 * 256_136);
    assert_eq!(de.size_mb, de.size_bytes / (1024 * 1024));
    assert_eq!((de.bundled_voices, de.bundled_voices_present), (13, 0));
    assert_eq!((en.bundled_voices, en.bundled_voices_present), (37, 0));
    assert_eq!(rt.bundled_voices, 0);

    // Ohne Programm fuer das System: keine Programmzeile, Modelle ohne Voraussetzung.
    let rows = build_downloads(&dirs, None, |id| id == "phonon-40m-de");
    assert!(rows.iter().all(|r| r.kind != TtsDownloadKind::Runtime));
    let de = rows.iter().find(|r| r.id == "phonon-40m-de").unwrap();
    assert!(de.requires.is_none());
    assert!(de.is_downloading, "der laufende Zustand wird durchgereicht");
}

/// Katalogeintrag mit kleinen Dateien (kein Netz) fuer Zustandstests.
fn tiny_entry(id: &str, files: &[(&str, &[u8])]) -> TtsCatalogEntry {
    TtsCatalogEntry {
        id: id.to_string(),
        name: "Phonon Test".to_string(),
        description: "Testmodell".to_string(),
        files: files
            .iter()
            .map(|(name, body)| catalog::TtsCatalogFile {
                filename: name.to_string(),
                url: format!("https://example.invalid/{name}"),
                size_bytes: body.len() as u64,
                sha256: Some(sha_hex(body)),
            })
            .collect(),
        tags: Vec::new(),
        license: Some("CC-BY-NC-4.0".to_string()),
        license_url: Some("https://example.org/lizenz".to_string()),
        license_non_commercial: true,
        unsupported_reason: None,
        license_note: Some("Hinweis".to_string()),
        languages: vec!["de".to_string()],
        revision: Some("abc123".to_string()),
        gated: false,
        license_verified_on: Some("2026-10-27".to_string()),
        source: Some("hf://gradium/phonon-40m-de@abc123".to_string()),
        parent: None,
        voice: None,
        file_git_blobs: BTreeMap::new(),
    }
}

fn write_model_files(dir: &Path, entry: &TtsCatalogEntry, files: &[(&str, &[u8])]) {
    fs::create_dir_all(dir).unwrap();
    for (name, body) in files {
        fs::write(dir.join(name), body).unwrap();
    }
    write_marker(dir, entry).unwrap();
}

#[test]
fn ein_modell_gilt_nur_mit_marke_und_allen_dateien_in_richtiger_groesse_als_installiert() {
    let tmp = TempDir::new().unwrap();
    let dirs = PhononDirs::new(tmp.path());
    let files: [(&str, &[u8]); 2] = [("a.json", b"abc"), ("b.gguf", b"12345")];
    let entry = tiny_entry("phonon-test", &files);
    let dir = dirs.model_dir("phonon-test");
    assert!(!model_complete(&dirs, &entry), "nichts da");

    write_model_files(&dir, &entry, &files);
    assert!(model_complete(&dirs, &entry));

    // Eine Datei fehlt.
    fs::remove_file(dir.join("b.gguf")).unwrap();
    assert!(!model_complete(&dirs, &entry));
    // Falsche Groesse (abgeschnitten).
    fs::write(dir.join("b.gguf"), b"123").unwrap();
    assert!(!model_complete(&dirs, &entry));
    fs::write(dir.join("b.gguf"), b"12345").unwrap();
    assert!(model_complete(&dirs, &entry));
    // Marke einer anderen Fassung (Katalog zeigt auf einen neueren Commit).
    let mut newer = entry.clone();
    newer.revision = Some("def456".into());
    assert!(!model_complete(&dirs, &newer));
    // Ohne Marke (von Hand abgelegt): nicht ueber die Modellseite installiert.
    fs::remove_file(dir.join(MARKER_FILE)).unwrap();
    assert!(!model_complete(&dirs, &entry));
}

#[test]
fn programm_und_modell_der_liste_folgen_dem_zustand_auf_der_platte() {
    let Some(platform) = current_platform() else {
        return;
    };
    let tmp = TempDir::new().unwrap();
    let dirs = PhononDirs::new(tmp.path());
    let rt = runtime_entry(platform).unwrap();
    let rt_dir = dirs.runtime_dir(platform);
    fs::create_dir_all(&rt_dir).unwrap();
    // Nur der Ordner, ohne Programm und Marke: nicht installiert.
    let rows = build_downloads(&dirs, Some(platform), |_| false);
    assert!(!rows[0].is_downloaded);
    // Programm ohne Marke (von Hand): auch nicht ueber die Modellseite installiert.
    fs::write(rt_dir.join(exe_name(platform)), b"exe").unwrap();
    assert!(!build_downloads(&dirs, Some(platform), |_| false)[0].is_downloaded);
    write_marker(&rt_dir, &rt).unwrap();
    let rows = build_downloads(&dirs, Some(platform), |_| false);
    assert!(rows[0].is_downloaded && rows[0].is_usable);
}

// ---- Aufloesung "installiert" --------------------------------------------------------------------

fn make_model(dirs: &PhononDirs, id: &str) {
    let entry = model_entry(id).unwrap();
    let dir = dirs.model_dir(id);
    fs::create_dir_all(&dir).unwrap();
    for f in &entry.files {
        // set_len legt die Datei in der richtigen Groesse an, ohne sie zu beschreiben.
        fs::File::create(dir.join(&f.filename))
            .unwrap()
            .set_len(f.size_bytes)
            .unwrap();
    }
    write_marker(&dir, &entry).unwrap();
}

#[test]
fn installierte_pfade_nennen_programm_und_das_deutsche_modell_zuerst() {
    let Some(platform) = current_platform() else {
        return;
    };
    let tmp = TempDir::new().unwrap();
    let dirs = PhononDirs::new(tmp.path());
    assert_eq!(
        installed_paths(tmp.path()),
        InstalledPaths::default(),
        "nichts installiert"
    );

    // Programm.
    let rt = runtime_entry(platform).unwrap();
    let rt_dir = dirs.runtime_dir(platform);
    fs::create_dir_all(&rt_dir).unwrap();
    fs::write(rt_dir.join(exe_name(platform)), b"exe").unwrap();
    write_marker(&rt_dir, &rt).unwrap();
    let p = installed_paths(tmp.path());
    let exe = p.phonon.exe_path.expect("Programm installiert");
    assert!(exe.ends_with(exe_name(platform)), "{exe}");
    assert!(Path::new(&exe).is_file());
    assert!(p.phonon.model.is_none());

    // Nur das englische Modell: das ist die zweite Wahl.
    make_model(&dirs, "phonon-31m-en");
    let p = installed_paths(tmp.path());
    assert!(p
        .phonon
        .model
        .as_deref()
        .unwrap()
        .ends_with("phonon-31m-en"));
    // Mit dem deutschen: das deutsche zuerst.
    make_model(&dirs, "phonon-40m-de");
    let p = installed_paths(tmp.path());
    assert!(p
        .phonon
        .model
        .as_deref()
        .unwrap()
        .ends_with("phonon-40m-de"));
    // Ein unvollstaendiges deutsches Modell zaehlt nicht.
    fs::remove_file(dirs.model_dir("phonon-40m-de").join("config.json")).unwrap();
    let p = installed_paths(tmp.path());
    assert!(p
        .phonon
        .model
        .as_deref()
        .unwrap()
        .ends_with("phonon-31m-en"));
}

#[test]
fn die_lizenz_installierter_katalogmodelle_kommt_aus_dem_katalog() {
    let tmp = TempDir::new().unwrap();
    let dirs = PhononDirs::new(tmp.path());
    make_model(&dirs, "phonon-40m-de");
    make_model(&dirs, "phonon-31m-en");

    let de = installed_model_license(&dirs.model_dir("phonon-40m-de").to_string_lossy()).unwrap();
    assert!(de.verified && de.non_commercial);
    assert_eq!(de.verified_on.as_deref(), Some("2026-10-27"));
    assert_eq!(
        de.source_url,
        "https://huggingface.co/gradium/phonon-40m-de"
    );
    assert!(de.weights.contains("CC-BY-NC-4.0"));
    assert!(de.note.contains("79cfc7b4") && de.note.contains("Modellseite"));
    assert_eq!(de.code, "MIT OR Apache-2.0");

    let en = installed_model_license(&dirs.model_dir("phonon-31m-en").to_string_lossy()).unwrap();
    assert!(en.verified && !en.non_commercial);
    assert!(en.weights.contains("Apache-2.0"));
}

#[test]
fn fremde_ordner_und_verweise_bekommen_keine_katalog_lizenz() {
    let tmp = TempDir::new().unwrap();
    let dirs = PhononDirs::new(tmp.path());
    // Richtiger Ort, aber keine Marke (von Hand abgelegte Gewichte gleichen Namens).
    let dir = dirs.model_dir("phonon-40m-de");
    fs::create_dir_all(&dir).unwrap();
    assert!(installed_model_license(&dir.to_string_lossy()).is_none());
    // Marke einer anderen Fassung.
    let mut other = model_entry("phonon-40m-de").unwrap();
    other.revision = Some("ffffffff".into());
    write_marker(&dir, &other).unwrap();
    assert!(installed_model_license(&dir.to_string_lossy()).is_none());
    // Falscher Ort (nicht .../tts/phonon/models/<id>), auch mit Marke.
    let elsewhere = tmp.path().join("meine-modelle").join("phonon-40m-de");
    write_model_files(&elsewhere, &model_entry("phonon-40m-de").unwrap(), &[]);
    assert!(installed_model_license(&elsewhere.to_string_lossy()).is_none());
    // Unbekannte Kennung, hf://-Verweis, leer.
    let unknown = dirs.model_dir("irgendein-modell");
    fs::create_dir_all(&unknown).unwrap();
    assert!(installed_model_license(&unknown.to_string_lossy()).is_none());
    assert!(installed_model_license("hf://gradium/phonon-40m-de").is_none());
    assert!(installed_model_license("").is_none());
}

// ---- Token ---------------------------------------------------------------------------------------------

fn env_map(pairs: &[(&str, String)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    move |k| map.get(k).cloned()
}

fn expose(s: Option<Secret>) -> Option<String> {
    s.map(|s| s.expose_for_test().to_string())
}

#[test]
fn das_token_kommt_aus_hf_token_dann_aus_der_datei() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().to_string_lossy().into_owned();

    // Umgebung zuerst, auch wenn eine Datei da ist.
    let hf_home = tmp.path().join("hf");
    fs::create_dir_all(&hf_home).unwrap();
    fs::write(hf_home.join("token"), "hf_ausDatei\n").unwrap();
    let env = env_map(&[
        ("HF_TOKEN", "  hf_ausUmgebung \n".to_string()),
        ("HF_HOME", hf_home.to_string_lossy().into_owned()),
    ]);
    assert_eq!(expose(hf_token(&env)).as_deref(), Some("hf_ausUmgebung"));

    // Ein kaputter HF_TOKEN faellt auf die Datei zurueck.
    let env = env_map(&[
        ("HF_TOKEN", "hat leerzeichen".to_string()),
        ("HF_HOME", hf_home.to_string_lossy().into_owned()),
    ]);
    assert_eq!(expose(hf_token(&env)).as_deref(), Some("hf_ausDatei"));

    // HF_HOME/token, sonst XDG_CACHE_HOME/huggingface/token, sonst ~/.cache/huggingface/token.
    let xdg = tmp.path().join("xdg");
    fs::create_dir_all(xdg.join("huggingface")).unwrap();
    fs::write(xdg.join("huggingface").join("token"), "\n\nhf_xdg\n").unwrap();
    let env = env_map(&[("XDG_CACHE_HOME", xdg.to_string_lossy().into_owned())]);
    assert_eq!(
        expose(hf_token(&env)).as_deref(),
        Some("hf_xdg"),
        "fuehrende Leerzeilen sind egal"
    );

    let cache = tmp.path().join(".cache").join("huggingface");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("token"), "hf_standardort").unwrap();
    for key in ["HOME", "USERPROFILE"] {
        let env = env_map(&[(key, home.clone())]);
        assert_eq!(
            expose(hf_token(&env)).as_deref(),
            Some("hf_standardort"),
            "{key}"
        );
    }

    // Nichts da, leer, unbrauchbar: kein Token.
    assert!(hf_token(&env_map(&[])).is_none());
    let empty = TempDir::new().unwrap();
    assert!(hf_token(&env_map(&[(
        "HOME",
        empty.path().to_string_lossy().into_owned()
    )]))
    .is_none());
    fs::write(cache.join("token"), "zwei woerter hier").unwrap();
    assert!(hf_token(&env_map(&[("HOME", home.clone())])).is_none());
    fs::write(cache.join("token"), [0xff, 0xfe, 0x00]).unwrap();
    assert!(
        hf_token(&env_map(&[("HOME", home)])).is_none(),
        "kein UTF-8"
    );
}

// ---- SHA-1 / Git-Blob -------------------------------------------------------------------------------------

#[test]
fn sha1_und_git_blob_entsprechen_den_bekannten_werten() {
    let sha = |data: &[u8]| {
        let mut s = Sha1::new();
        s.update(data);
        s.finish()
    };
    assert_eq!(sha(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(sha(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(
        sha(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
    );
    assert_eq!(
        sha(&[b'a'; 200]),
        "e61cfffe0d9195a525fc6cf06ca2d77119c24a40"
    );
    // In Stuecken gefuettert (ueber die Blockgrenze) gleich wie am Stueck.
    let mut chunked = Sha1::new();
    for part in [&[b'a'; 7][..], &[b'a'; 60], &[b'a'; 64], &[b'a'; 69]] {
        chunked.update(part);
    }
    assert_eq!(chunked.finish(), "e61cfffe0d9195a525fc6cf06ca2d77119c24a40");

    // Git-Blob von "hello\n": `git hash-object` liefert ce013625...
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("hello.txt");
    fs::write(&p, b"hello\n").unwrap();
    assert_eq!(
        git_blob_sha1(&p).unwrap(),
        "ce013625030ba8dba906f756967f9e9ca394464a"
    );
    assert!(verify_git_blob(&p, "ce013625030ba8dba906f756967f9e9ca394464a", "hello.txt").is_ok());
    assert!(p.exists());
    let err = verify_git_blob(&p, &"0".repeat(40), "hello.txt").unwrap_err();
    assert!(err.contains("beschädigt"), "{err}");
    assert!(!p.exists(), "die abweichende Datei wird geloescht");
}

// ---- Entpacken ------------------------------------------------------------------------------------------------

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts: zip::write::SimpleFileOptions = Default::default();
        for (name, data) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }
    buf.into_inner()
}

fn tar_gz_bytes(entries: &[(&str, &[u8], tar::EntryType)]) -> Vec<u8> {
    let mut raw = Vec::new();
    {
        let enc = flate2::write::GzEncoder::new(&mut raw, flate2::Compression::fast());
        let mut builder = tar::Builder::new(enc);
        for (name, data, kind) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(if kind.is_file() { data.len() as u64 } else { 0 });
            h.set_entry_type(*kind);
            h.set_mode(0o644);
            // Der Name wird roh gesetzt: `set_path` wuerde `..` selbst ablehnen.
            h.as_old_mut().name = [0; 100];
            h.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
            if !kind.is_file() && !kind.is_dir() {
                h.set_link_name("/etc/passwd").unwrap();
            }
            h.set_cksum();
            let body: &[u8] = if kind.is_file() { data } else { &[] };
            builder.append(&h, body).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
    }
    raw
}

fn runtime_fixture_entry() -> TtsCatalogEntry {
    let mut e = tiny_entry("phonon-runtime-windows-x64", &[("p.zip", b"x")]);
    e.revision = Some("v0.4.2".into());
    e
}

#[test]
fn zip_wird_entpackt_nur_die_noetigen_dateien_kommen_mit() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("windows-x64.download");
    fs::write(
        &archive,
        zip_bytes(&[
            ("ptts-0.4.2-x/LICENSE-APACHE", b"apache"),
            ("ptts-0.4.2-x/LICENSE-MIT", b"mit"),
            ("ptts-0.4.2-x/ptts.exe", b"EXE-BYTES"),
            ("ptts-0.4.2-x/README.md", b"readme"),
            ("ptts-0.4.2-x/docs/tief/zu-tief.bin", b"tief"), // zu tief: ignoriert
            ("ptts-0.4.2-x/anderes.dll", b"dll"),            // nicht benannt: ignoriert
            ("nebenher.txt", b"x"),                          // nicht benannt: ignoriert
        ]),
    )
    .unwrap();
    let dest = tmp.path().join("windows-x64");
    let entry = runtime_fixture_entry();

    extract_ptts_archive(&archive, &dest, true, "windows-x64", &entry).unwrap();

    assert_eq!(fs::read(dest.join("ptts.exe")).unwrap(), b"EXE-BYTES");
    assert_eq!(fs::read(dest.join("LICENSE-MIT")).unwrap(), b"mit");
    assert_eq!(fs::read(dest.join("LICENSE-APACHE")).unwrap(), b"apache");
    assert!(dest.join("README.md").is_file());
    let mut names: Vec<String> = fs::read_dir(&dest)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            ".lva-install.json",
            "LICENSE-APACHE",
            "LICENSE-MIT",
            "README.md",
            "ptts.exe"
        ],
        "genau die benannten Dateien plus die Marke"
    );
    assert!(marker_matches(&dest, &entry));
    assert!(!tmp.path().join("windows-x64.extracting").exists());
}

#[test]
fn zip_mit_unzulaessigem_pfad_wird_abgelehnt_ohne_etwas_auszuserhalb_zu_schreiben() {
    for bad in [
        "../evil.txt",
        "..\\evil.txt",
        "a/../../evil.txt",
        "/abs/evil.txt",
        "\\\\server\\share\\evil.txt",
        "C:/evil.txt",
        "ptts.exe:stream",
    ] {
        let tmp = TempDir::new().unwrap();
        let sandbox = tmp.path().join("sandbox");
        fs::create_dir_all(&sandbox).unwrap();
        let archive = sandbox.join("a.download");
        fs::write(
            &archive,
            zip_bytes(&[("ptts.exe", b"EXE"), (bad, b"boese")]),
        )
        .unwrap();
        let dest = sandbox.join("windows-x64");

        let err = extract_ptts_archive(
            &archive,
            &dest,
            true,
            "windows-x64",
            &runtime_fixture_entry(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("unzulässigen Pfad"),
            "{bad}: {err}"
        );
        assert!(!dest.exists(), "{bad}: nichts wird eingesetzt");
        assert!(
            !sandbox.join("windows-x64.extracting").exists(),
            "{bad}: aufgeraeumt"
        );
        assert!(
            !tmp.path().join("evil.txt").exists(),
            "{bad}: nichts ausserhalb"
        );
        assert!(!sandbox.join("evil.txt").exists());
    }
}

#[test]
fn tar_gz_wird_entpackt_und_verknuepfungen_oder_pfade_nach_aussen_werden_abgelehnt() {
    use tar::EntryType::{Directory, Regular, Symlink};
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("macos-aarch64");
    let archive = tmp.path().join("m.download");
    fs::write(
        &archive,
        tar_gz_bytes(&[
            ("ptts-0.4.2-x", b"", Directory),
            ("ptts-0.4.2-x/LICENSE-MIT", b"mit", Regular),
            ("ptts-0.4.2-x/ptts", b"BIN", Regular),
            ("ptts-0.4.2-x/README.md", b"rd", Regular),
            ("ptts-0.4.2-x/fremd.so", b"so", Regular),
        ]),
    )
    .unwrap();
    let entry = runtime_fixture_entry();
    extract_ptts_archive(&archive, &dest, false, "macos-aarch64", &entry).unwrap();
    assert_eq!(fs::read(dest.join("ptts")).unwrap(), b"BIN");
    assert!(dest.join("LICENSE-MIT").is_file() && dest.join("README.md").is_file());
    assert!(!dest.join("fremd.so").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dest.join("ptts"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "ausfuehrbar");
    }

    // Verknuepfung unter dem Namen des Programms.
    let sandbox = tmp.path().join("s2");
    fs::create_dir_all(&sandbox).unwrap();
    let link = sandbox.join("l.download");
    fs::write(&link, tar_gz_bytes(&[("top/ptts", b"", Symlink)])).unwrap();
    let err = extract_ptts_archive(
        &link,
        &sandbox.join("linux-x64"),
        false,
        "linux-x64",
        &entry,
    )
    .unwrap_err();
    assert!(err.to_string().contains("Verknüpfung"), "{err}");
    assert!(!sandbox.join("linux-x64").exists());

    // Pfad nach aussen.
    let evil = sandbox.join("e.download");
    fs::write(
        &evil,
        tar_gz_bytes(&[
            ("top/ptts", b"BIN", Regular),
            ("../evil.txt", b"x", Regular),
        ]),
    )
    .unwrap();
    let err = extract_ptts_archive(
        &evil,
        &sandbox.join("linux-x64"),
        false,
        "linux-x64",
        &entry,
    )
    .unwrap_err();
    assert!(err.to_string().contains("unzulässigen Pfad"), "{err}");
    assert!(!tmp.path().join("evil.txt").exists() && !sandbox.join("evil.txt").exists());
    assert!(!sandbox.join("linux-x64").exists());
}

#[test]
fn ein_archiv_mit_doppeltem_programm_wird_abgelehnt() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("d.download");
    fs::write(
        &archive,
        zip_bytes(&[("a/ptts.exe", b"1"), ("b/ptts.exe", b"2")]),
    )
    .unwrap();
    let err = extract_ptts_archive(
        &archive,
        &tmp.path().join("w"),
        true,
        "windows-x64",
        &runtime_fixture_entry(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("doppelt"), "{err}");
}

#[test]
fn eine_zu_grosse_datei_im_archiv_wird_abgelehnt() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("g.download");
    let big = vec![b'x'; (MAX_SMALL_BYTES + 1) as usize];
    fs::write(
        &archive,
        zip_bytes(&[("ptts.exe", b"EXE"), ("README.md", &big)]),
    )
    .unwrap();
    let err = extract_ptts_archive(
        &archive,
        &tmp.path().join("w"),
        true,
        "windows-x64",
        &runtime_fixture_entry(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("unplausibel groß"), "{err}");
    assert!(!tmp.path().join("w").exists());
}

#[test]
fn unvollstaendiges_archiv_laesst_die_vorhandene_installation_unberuehrt() {
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("windows-x64");
    fs::create_dir_all(&dest).unwrap();
    fs::write(dest.join("ptts.exe"), b"alt").unwrap();
    // Archiv ohne das Programm.
    let archive = tmp.path().join("u.download");
    fs::write(&archive, zip_bytes(&[("top/README.md", b"nur readme")])).unwrap();

    let err = extract_ptts_archive(
        &archive,
        &dest,
        true,
        "windows-x64",
        &runtime_fixture_entry(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("ptts.exe nicht"), "{err}");
    assert_eq!(fs::read(dest.join("ptts.exe")).unwrap(), b"alt");
    assert!(!tmp.path().join("windows-x64.extracting").exists());
    assert!(!tmp.path().join("windows-x64.old").exists());

    // Ein defektes Archiv ebenso.
    fs::write(&archive, b"kein zip").unwrap();
    assert!(extract_ptts_archive(
        &archive,
        &dest,
        true,
        "windows-x64",
        &runtime_fixture_entry()
    )
    .is_err());
    assert_eq!(fs::read(dest.join("ptts.exe")).unwrap(), b"alt");
}

#[test]
fn ersetzen_stellt_bei_einem_fehler_die_alte_fassung_wieder_her() {
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("ziel");
    fs::create_dir_all(&dest).unwrap();
    fs::write(dest.join("alt.txt"), b"alt").unwrap();

    // Die neue Fassung existiert nicht: das Umbenennen scheitert, die alte kommt zurueck.
    assert!(replace_dir(&tmp.path().join("gibt-es-nicht"), &dest).is_err());
    assert_eq!(fs::read(dest.join("alt.txt")).unwrap(), b"alt");
    assert!(!tmp.path().join("ziel.old").exists());

    // Gelingt es, ist die neue Fassung da und die Sicherung weg.
    let new_dir = tmp.path().join("neu");
    fs::create_dir_all(&new_dir).unwrap();
    fs::write(new_dir.join("neu.txt"), b"neu").unwrap();
    replace_dir(&new_dir, &dest).unwrap();
    assert!(dest.join("neu.txt").is_file() && !dest.join("alt.txt").exists());
    assert!(!tmp.path().join("ziel.old").exists());
}

// ---- Fake-Server und Harness ---------------------------------------------------------------------------------------

#[derive(Clone)]
enum Reply {
    Body(Vec<u8>),
    Status(u16),
    Redirect(String),
}

#[derive(Debug, Clone)]
struct Hit {
    path: String,
    auth: Option<String>,
    range: Option<String>,
}

struct FakeHost {
    base: String,
    port: u16,
    hits: Arc<Mutex<Vec<Hit>>>,
}

impl FakeHost {
    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }

    fn count(&self, path: &str) -> usize {
        self.hits().iter().filter(|h| h.path == path).count()
    }

    fn origin(&self) -> Origin {
        Origin::plain_http("127.0.0.1", self.port)
    }
}

async fn fake_host(routes: Vec<(&str, Reply)>) -> FakeHost {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let routes: Arc<HashMap<String, Reply>> = Arc::new(
        routes
            .into_iter()
            .map(|(p, r)| (p.to_string(), r))
            .collect(),
    );
    let hits = Arc::new(Mutex::new(Vec::new()));
    let (routes_task, hits_task) = (routes.clone(), hits.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let (routes, hits) = (routes_task.clone(), hits_task.clone());
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match sock.read(&mut byte).await {
                        Ok(0) | Err(_) => return,
                        Ok(_) => head.push(byte[0]),
                    }
                }
                let text = String::from_utf8_lossy(&head).into_owned();
                let path = text
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let header = |name: &str| {
                    text.lines().find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.trim()
                            .eq_ignore_ascii_case(name)
                            .then(|| v.trim().to_string())
                    })
                };
                let (auth, range) = (header("authorization"), header("range"));
                hits.lock().unwrap().push(Hit {
                    path: path.clone(),
                    auth,
                    range: range.clone(),
                });
                let response = match routes.get(&path) {
                    Some(Reply::Body(b)) => {
                        let start = range
                            .as_deref()
                            .and_then(|r| r.strip_prefix("bytes="))
                            .and_then(|r| r.split('-').next())
                            .and_then(|n| n.parse::<usize>().ok())
                            .unwrap_or(0);
                        let mut out = if start > 0 {
                            format!(
                                "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nContent-Range: bytes {start}-{}/{}\r\nContent-Length: {}\r\n\r\n",
                                b.len() - 1,
                                b.len(),
                                b.len() - start
                            )
                        } else {
                            format!(
                                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                                b.len()
                            )
                        }
                        .into_bytes();
                        out.extend_from_slice(&b[start..]);
                        out
                    }
                    Some(Reply::Status(code)) => format!(
                        "HTTP/1.1 {code} Fehler\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
                    )
                    .into_bytes(),
                    Some(Reply::Redirect(loc)) => format!(
                        "HTTP/1.1 302 Found\r\nConnection: close\r\nLocation: {loc}\r\nContent-Length: 0\r\n\r\n"
                    )
                    .into_bytes(),
                    None => b"HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
                        .to_vec(),
                };
                let _ = sock.write_all(&response).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    FakeHost {
        base: format!("http://127.0.0.1:{port}"),
        port,
        hits,
    }
}

struct Harness {
    _tmp: TempDir,
    dirs: PhononDirs,
    cancel: CancellationToken,
    lock: Mutex<()>,
    emit: Box<dyn Fn(HttpDownloadEvent<'_>) + Send + Sync>,
    token: Box<dyn Fn() -> Option<Secret> + Send + Sync>,
    progress: Arc<Mutex<Vec<(u64, u64)>>>,
    verified: Arc<AtomicUsize>,
}

impl Harness {
    /// `cancel_after`: nach so vielen geprueften Dateien wird abgebrochen.
    fn new(token: Option<&'static str>, cancel_after: Option<usize>) -> Self {
        let tmp = TempDir::new().unwrap();
        let cancel = CancellationToken::new();
        let progress = Arc::new(Mutex::new(Vec::new()));
        let verified = Arc::new(AtomicUsize::new(0));
        let (p, v, c) = (progress.clone(), verified.clone(), cancel.clone());
        let emit = Box::new(move |event: HttpDownloadEvent<'_>| match event {
            HttpDownloadEvent::Progress(pr) => p.lock().unwrap().push((pr.downloaded, pr.total)),
            HttpDownloadEvent::VerificationStarted => {}
            HttpDownloadEvent::VerificationCompleted => {
                let n = v.fetch_add(1, Ordering::SeqCst) + 1;
                if cancel_after == Some(n) {
                    c.cancel();
                }
            }
        });
        let dirs = PhononDirs::new(tmp.path());
        Self {
            _tmp: tmp,
            dirs,
            cancel,
            lock: Mutex::new(()),
            emit,
            token: Box::new(move || token.and_then(Secret::new)),
            progress,
            verified,
        }
    }

    fn env(&self, origin: Origin) -> InstallEnv<'_> {
        InstallEnv {
            dirs: &self.dirs,
            platform: Some("windows-x64"),
            cancel: &self.cancel,
            emit: &*self.emit,
            event_id: "phonon-test",
            token: &*self.token,
            auth_origin: origin,
            voices_lock: &self.lock,
        }
    }
}

/// Modell mit drei Dateien auf dem Fake-Host: a (SHA-256), b (nur Git-Blob), c (beides).
fn model_on(host: &FakeHost, gated: bool) -> (TtsCatalogEntry, Vec<(&'static str, Vec<u8>)>) {
    let bodies: Vec<(&'static str, Vec<u8>)> = vec![
        ("config.json", b"{\"a\":1}".to_vec()),
        ("tokenizer.json", b"{\"tokens\":[1,2,3]}".to_vec()),
        ("model.q8.gguf", vec![7u8; 5000]),
    ];
    let mut entry = tiny_entry("phonon-test", &[]);
    entry.gated = gated;
    entry.files = bodies
        .iter()
        .map(|(name, body)| catalog::TtsCatalogFile {
            filename: name.to_string(),
            url: format!("{}/files/{name}", host.base),
            size_bytes: body.len() as u64,
            sha256: (*name != "tokenizer.json").then(|| sha_hex(body)),
        })
        .collect();
    entry
        .file_git_blobs
        .insert("tokenizer.json".into(), git_blob_of(&bodies[1].1));
    entry
        .file_git_blobs
        .insert("config.json".into(), git_blob_of(&bodies[0].1));
    (entry, bodies)
}

fn routes_for(bodies: &[(&'static str, Vec<u8>)]) -> Vec<(String, Reply)> {
    bodies
        .iter()
        .map(|(n, b)| (format!("/files/{n}"), Reply::Body(b.clone())))
        .collect()
}

async fn host_for(bodies: &[(&'static str, Vec<u8>)]) -> FakeHost {
    let routes = routes_for(bodies);
    fake_host(
        routes
            .iter()
            .map(|(p, r)| (p.as_str(), r.clone()))
            .collect(),
    )
    .await
}

// ---- Installieren: Modell ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn ein_oeffentliches_modell_wird_geladen_geprueft_und_atomar_eingesetzt() {
    let h = Harness::new(None, None);
    let (_, bodies) = model_on(&fake_host(vec![]).await, false);
    let host = host_for(&bodies).await;
    let (entry, bodies) = model_on(&host, false);

    let out = install_model(&h.env(host.origin()), &entry).await.unwrap();

    assert_eq!(out, Outcome::Installed);
    let dir = h.dirs.model_dir("phonon-test");
    for (name, body) in &bodies {
        assert_eq!(&fs::read(dir.join(name)).unwrap(), body, "{name}");
    }
    assert!(model_complete(&h.dirs, &entry));
    assert!(marker_matches(&dir, &entry));
    assert!(
        !h.dirs.staging_dir("phonon-test").exists(),
        "Zwischenordner ist weg"
    );
    assert!(
        host.hits().iter().all(|x| x.auth.is_none()),
        "oeffentlich: nie ein Token"
    );
    // Ein Fortschritt ueber alle Dateien: nie rueckwaerts, am Ende voll, Summe = alle Dateien.
    let total: u64 = bodies.iter().map(|(_, b)| b.len() as u64).sum();
    let progress = h.progress.lock().unwrap().clone();
    assert!(progress.iter().all(|(_, t)| *t == total), "{progress:?}");
    assert!(
        progress.windows(2).all(|w| w[0].0 <= w[1].0),
        "{progress:?}"
    );
    assert_eq!(progress.last().copied(), Some((total, total)));
    assert_eq!(
        h.verified.load(Ordering::SeqCst),
        3,
        "jede der drei Dateien wird geprueft"
    );
}

#[tokio::test]
async fn ein_gesperrtes_modell_sendet_das_token_nur_an_den_freigegebenen_ursprung() {
    let h = Harness::new(Some(TOKEN), None);
    let (_, bodies) = model_on(&fake_host(vec![]).await, true);
    let host = host_for(&bodies).await;
    let (entry, _) = model_on(&host, true);

    install_model(&h.env(host.origin()), &entry).await.unwrap();

    let hits = host.hits();
    assert_eq!(hits.len(), 3);
    for hit in &hits {
        assert_eq!(
            hit.auth.as_deref(),
            Some(format!("Bearer {TOKEN}").as_str()),
            "{hit:?}"
        );
    }
    assert!(model_complete(&h.dirs, &entry));

    // Passt der freigegebene Ursprung nicht auf den Host (hier: huggingface.co), geht nichts hinaus.
    let h2 = Harness::new(Some(TOKEN), None);
    install_model(&h2.env(Origin::huggingface()), &entry)
        .await
        .unwrap();
    let hits = host.hits();
    assert_eq!(hits.len(), 6);
    assert!(
        hits[3..].iter().all(|x| x.auth.is_none()),
        "Token an einen nicht freigegebenen Host: {:?}",
        &hits[3..]
    );
}

#[tokio::test]
async fn bei_einer_weiterleitung_zum_cdn_bekommt_das_cdn_kein_token() {
    let (_, bodies) = model_on(&fake_host(vec![]).await, true);
    let cdn = host_for(&bodies).await;
    // Der Ursprung leitet jede Datei auf das CDN um.
    let redirects: Vec<(String, Reply)> = bodies
        .iter()
        .map(|(n, _)| {
            (
                format!("/files/{n}"),
                Reply::Redirect(format!("{}/files/{n}", cdn.base)),
            )
        })
        .collect();
    let origin = fake_host(
        redirects
            .iter()
            .map(|(p, r)| (p.as_str(), r.clone()))
            .collect(),
    )
    .await;
    let (entry, _) = model_on(&origin, true);
    let h = Harness::new(Some(TOKEN), None);

    install_model(&h.env(origin.origin()), &entry)
        .await
        .unwrap();

    assert!(
        origin.hits().iter().all(|x| x.auth.is_some()),
        "der Ursprung bekommt es"
    );
    assert_eq!(cdn.hits().len(), 3);
    assert!(
        cdn.hits().iter().all(|x| x.auth.is_none()),
        "das CDN darf es nie sehen: {:?}",
        cdn.hits()
    );
    assert!(model_complete(&h.dirs, &entry));
}

#[tokio::test]
async fn ein_gesperrtes_modell_ohne_token_nennt_den_weg_und_laedt_nichts() {
    let h = Harness::new(None, None);
    let host = fake_host(vec![]).await;
    let (mut entry, _) = model_on(&host, true);
    entry.name = "Phonon 40m (German, q8)".into();

    let err = install_model(&h.env(host.origin()), &entry)
        .await
        .unwrap_err();

    assert!(
        err.contains("Bedingungen auf huggingface.co/gradium/phonon-40m-de annehmen und anmelden (hf auth login)"),
        "{err}"
    );
    assert!(err.contains("HF_TOKEN"));
    assert!(host.hits().is_empty(), "kein einziger Aufruf");
    assert!(!h.dirs.model_dir("phonon-test").exists());
    assert!(
        !h.dirs.staging_dir("phonon-test").exists(),
        "nicht einmal ein Zwischenordner"
    );
}

#[tokio::test]
async fn http_401_und_403_ergeben_den_hinweis_und_nennen_das_token_nie() {
    for status in [401u16, 403] {
        let host = fake_host(vec![
            ("/files/config.json", Reply::Status(status)),
            ("/files/tokenizer.json", Reply::Status(status)),
            ("/files/model.q8.gguf", Reply::Status(status)),
        ])
        .await;
        let (entry, _) = model_on(&host, true);
        let h = Harness::new(Some(TOKEN), None);

        let err = install_model(&h.env(host.origin()), &entry)
            .await
            .unwrap_err();

        assert!(
            err.contains("Bedingungen auf huggingface.co/gradium/phonon-40m-de annehmen und anmelden (hf auth login)"),
            "{status}: {err}"
        );
        assert!(!err.contains(TOKEN), "{status}: Token im Fehlertext: {err}");
        assert!(!err.to_lowercase().contains("bearer"), "{err}");
        assert!(!model_complete(&h.dirs, &entry));
        assert!(
            !h.dirs.model_dir("phonon-test").exists(),
            "kein halbes Modell am Zielort"
        );
        assert!(
            host.hits().iter().all(|x| x.auth.is_some()),
            "angefragt wurde mit Token"
        );

        // Ein oeffentliches Modell mit 403 bekommt keinen Anmelde-Hinweis, und es geht nie ein Token hin.
        let (public, _) = model_on(&host, false);
        let h = Harness::new(Some(TOKEN), None);
        let err = install_model(&h.env(host.origin()), &public)
            .await
            .unwrap_err();
        assert!(
            err.contains("verweigert") && !err.contains("Bedingungen"),
            "{err}"
        );
        assert!(host.hits().last().unwrap().auth.is_none());
    }
}

#[tokio::test]
async fn eine_abweichende_sha256_installiert_nichts_und_loescht_die_datei() {
    let (_, bodies) = model_on(&fake_host(vec![]).await, false);
    // Der Server liefert fuer model.q8.gguf andere Bytes (gleiche Laenge) als der Katalog verspricht.
    let mut served = bodies.clone();
    served[2].1 = vec![8u8; 5000];
    let host = host_for(&served).await;
    let (entry, _) = model_on(&host, false);
    let h = Harness::new(None, None);

    let err = install_model(&h.env(host.origin()), &entry)
        .await
        .unwrap_err();

    assert!(
        err.contains("corrupt") || err.contains("beschädigt"),
        "{err}"
    );
    assert!(!model_complete(&h.dirs, &entry));
    assert!(!h.dirs.model_dir("phonon-test").exists());
    let staging = h.dirs.staging_dir("phonon-test");
    assert!(
        !staging.join("model.q8.gguf").exists(),
        "die beschaedigte Datei ist weg"
    );
    assert!(!staging.join("model.q8.gguf.partial").exists());
    assert!(!staging.join(MARKER_FILE).exists(), "keine Marke");
}

#[tokio::test]
async fn eine_abweichende_git_blob_kennung_installiert_nichts() {
    let (_, bodies) = model_on(&fake_host(vec![]).await, false);
    let mut served = bodies.clone();
    served[1].1 = b"{\"tokens\":[9,9,9]}".to_vec(); // gleiche Laenge, anderer Inhalt
    let host = host_for(&served).await;
    let (entry, _) = model_on(&host, false);
    let h = Harness::new(None, None);

    let err = install_model(&h.env(host.origin()), &entry)
        .await
        .unwrap_err();

    assert!(
        err.contains("beschädigt") && err.contains("tokenizer.json"),
        "{err}"
    );
    assert!(!model_complete(&h.dirs, &entry));
    assert!(!h
        .dirs
        .staging_dir("phonon-test")
        .join("tokenizer.json")
        .exists());
}

#[tokio::test]
async fn abbruch_haelt_an_und_die_wiederaufnahme_laedt_fertige_dateien_nicht_neu() {
    let (_, bodies) = model_on(&fake_host(vec![]).await, false);
    let host = host_for(&bodies).await;
    let (entry, _) = model_on(&host, false);

    // Abbruch nach der ersten fertigen Datei.
    let h = Harness::new(None, Some(1));
    let out = install_model(&h.env(host.origin()), &entry).await.unwrap();
    assert_eq!(out, Outcome::Cancelled);
    assert!(
        !h.dirs.model_dir("phonon-test").exists(),
        "kein halbes Modell am Zielort"
    );
    assert!(!model_complete(&h.dirs, &entry));
    let staging = h.dirs.staging_dir("phonon-test");
    assert!(
        staging.join("config.json").is_file(),
        "die fertige Datei bleibt"
    );
    assert!(!staging.join(MARKER_FILE).exists());

    // Neuer Versuch im selben Ordner: nur der Rest wird geladen.
    let cancel = CancellationToken::new();
    let progress_cb = |_: HttpDownloadEvent<'_>| {};
    let token = || -> Option<Secret> { None };
    let lock = Mutex::new(());
    let env = InstallEnv {
        dirs: &h.dirs,
        platform: Some("windows-x64"),
        cancel: &cancel,
        emit: &progress_cb,
        event_id: "phonon-test",
        token: &token,
        auth_origin: host.origin(),
        voices_lock: &lock,
    };
    let out = install_model(&env, &entry).await.unwrap();
    assert_eq!(out, Outcome::Installed);
    assert!(model_complete(&h.dirs, &entry));
    assert_eq!(host.count("/files/config.json"), 1, "nicht erneut geladen");
    assert_eq!(host.count("/files/tokenizer.json"), 1);
    assert_eq!(host.count("/files/model.q8.gguf"), 1);
}

#[tokio::test]
async fn ein_schon_installiertes_modell_wird_nicht_erneut_geladen() {
    let (_, bodies) = model_on(&fake_host(vec![]).await, false);
    let host = host_for(&bodies).await;
    let (entry, _) = model_on(&host, false);
    let h = Harness::new(None, None);
    install_model(&h.env(host.origin()), &entry).await.unwrap();
    let before = host.hits().len();
    assert_eq!(
        install_model(&h.env(host.origin()), &entry).await.unwrap(),
        Outcome::Installed
    );
    assert_eq!(host.hits().len(), before);
}

#[tokio::test]
async fn eine_installation_ueber_einen_vorhandenen_ordner_behaelt_die_stimmen() {
    let (_, bodies) = model_on(&fake_host(vec![]).await, false);
    let host = host_for(&bodies).await;
    let (entry, _) = model_on(&host, false);
    let h = Harness::new(None, None);
    // Ein alter, unvollstaendiger Stand samt einer installierten Stimme.
    let dir = h.dirs.model_dir("phonon-test");
    fs::create_dir_all(dir.join("voices")).unwrap();
    fs::write(dir.join("voices").join("Nele.safetensors"), b"stimme").unwrap();
    fs::write(dir.join("config.json"), b"alt").unwrap();

    install_model(&h.env(host.origin()), &entry).await.unwrap();

    assert_eq!(
        fs::read(dir.join("voices").join("Nele.safetensors")).unwrap(),
        b"stimme"
    );
    assert_eq!(
        fs::read(dir.join("config.json")).unwrap(),
        b"{\"a\":1}",
        "alte Datei ersetzt"
    );
    assert!(model_complete(&h.dirs, &entry));
    assert!(!h.dirs.staging_dir("phonon-test").exists());
}

#[tokio::test]
async fn ein_nicht_beschreibbares_ziel_ist_eine_meldung_und_nichts_gilt_als_installiert() {
    let (_, bodies) = model_on(&fake_host(vec![]).await, false);
    let host = host_for(&bodies).await;
    let (entry, _) = model_on(&host, false);
    let h = Harness::new(None, None);
    // `models` ist eine Datei: kein Ordner anlegbar (so wie ein voller oder gesperrter Datentraeger).
    fs::create_dir_all(h.dirs.root()).unwrap();
    fs::write(h.dirs.models_dir(), b"ich bin kein ordner").unwrap();

    let err = install_model(&h.env(host.origin()), &entry)
        .await
        .unwrap_err();

    assert!(
        err.contains("Phonon Test") && err.contains("schreiben"),
        "{err}"
    );
    assert!(!model_complete(&h.dirs, &entry));
    assert!(host.hits().is_empty(), "ohne Ziel wird nichts geladen");
}

#[test]
fn meldungen_zu_fehlerstatus_und_vollem_datentraeger() {
    let entry = tiny_entry("phonon-test", &[]);
    let status = |code: u16| {
        anyhow::Error::new(HttpStatusError(
            reqwest::StatusCode::from_u16(code).unwrap(),
        ))
    };
    assert!(describe_error(&entry, &status(404)).contains("nicht gefunden"));
    assert!(describe_error(&entry, &status(500)).contains("HTTP 500"));
    assert!(describe_error(&entry, &status(401)).contains("verweigert"));
    let full = anyhow::Error::new(std::io::Error::from_raw_os_error(112));
    assert!(describe_error(&entry, &full).contains("nicht genug Speicherplatz"));
    let other = anyhow!("irgendwas");
    assert!(describe_error(&entry, &other).contains("irgendwas"));
}

#[test]
fn signierte_adressen_stehen_in_keiner_meldung() {
    let entry = tiny_entry("phonon-test", &[]);
    let err = anyhow!(
        "error decoding response body for url (https://cas-bridge.example/x?X-Amz-Signature=geheim123&y=2) und http://a.b/c?d=e Ende"
    );
    let msg = describe_error(&entry, &err);
    assert!(
        !msg.contains("geheim123") && !msg.contains("cas-bridge") && !msg.contains("http"),
        "{msg}"
    );
    assert!(msg.contains("<Adresse>") && msg.contains("Ende"), "{msg}");
    assert_eq!(scrub_urls("kein Link hier"), "kein Link hier");
    assert_eq!(scrub_urls("https://x.y"), "<Adresse>");
}

#[test]
fn eine_kennung_laesst_sich_nur_einmal_belegen() {
    let flags = Mutex::new(HashMap::new());
    let first = try_claim(&flags, "phonon-40m-de").expect("frei");
    assert!(
        try_claim(&flags, "phonon-40m-de").is_none(),
        "zweiter Start wird abgewiesen"
    );
    assert!(
        try_claim(&flags, "phonon-31m-en").is_some(),
        "andere Kennung ist frei"
    );
    // Der Abbruch der ersten wirkt auf ihr Token.
    flags.lock().unwrap().get("phonon-40m-de").unwrap().cancel();
    assert!(first.is_cancelled());
    flags.lock().unwrap().remove("phonon-40m-de");
    assert!(
        try_claim(&flags, "phonon-40m-de").is_some(),
        "nach dem Freigeben wieder frei"
    );
}

// ---- Installieren: Programm ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn das_programm_wird_geladen_geprueft_entpackt_und_das_archiv_entfernt() {
    let zip = zip_bytes(&[
        ("ptts-0.4.2-x/ptts.exe", b"EXE"),
        ("ptts-0.4.2-x/LICENSE-MIT", b"mit"),
    ]);
    let host = fake_host(vec![("/p.zip", Reply::Body(zip.clone()))]).await;
    let mut entry = tiny_entry("phonon-runtime-windows-x64", &[]);
    entry.revision = Some("v0.4.2".into());
    entry.files = vec![catalog::TtsCatalogFile {
        filename: "p.zip".into(),
        url: format!("{}/p.zip", host.base),
        size_bytes: zip.len() as u64,
        sha256: Some(sha_hex(&zip)),
    }];
    let h = Harness::new(None, None);

    let out = install_runtime_entry(&h.env(host.origin()), "windows-x64", &entry)
        .await
        .unwrap();

    assert_eq!(out, Outcome::Installed);
    let dest = h.dirs.runtime_dir("windows-x64");
    assert_eq!(fs::read(dest.join("ptts.exe")).unwrap(), b"EXE");
    assert!(runtime_installed(&h.dirs, "windows-x64", &entry));
    assert!(
        !h.dirs.root().join("windows-x64.download").exists(),
        "Archiv entfernt"
    );
    assert!(host.hits().iter().all(|x| x.auth.is_none()));
    assert_eq!(
        h.progress.lock().unwrap().last().copied(),
        Some((zip.len() as u64, zip.len() as u64))
    );
    // Noch einmal: nichts wird geladen.
    install_runtime_entry(&h.env(host.origin()), "windows-x64", &entry)
        .await
        .unwrap();
    assert_eq!(host.hits().len(), 1);
}

#[tokio::test]
async fn ein_falsches_programm_archiv_wird_nicht_installiert() {
    let zip = zip_bytes(&[("top/ptts.exe", b"EXE")]);
    let host = fake_host(vec![("/p.zip", Reply::Body(zip.clone()))]).await;
    let mut entry = tiny_entry("phonon-runtime-windows-x64", &[]);
    entry.files = vec![catalog::TtsCatalogFile {
        filename: "p.zip".into(),
        url: format!("{}/p.zip", host.base),
        size_bytes: zip.len() as u64,
        sha256: Some("0".repeat(64)), // Pruefsumme passt nicht
    }];
    let h = Harness::new(None, None);

    let err = install_runtime_entry(&h.env(host.origin()), "windows-x64", &entry)
        .await
        .unwrap_err();

    assert!(
        err.contains("corrupt") || err.contains("beschädigt"),
        "{err}"
    );
    assert!(!h.dirs.runtime_dir("windows-x64").exists());
    assert!(!h.dirs.root().join("windows-x64.download").exists());
}

// ---- Stimmen ----------------------------------------------------------------------------------------------------------------

fn voice_fixture(host: &FakeHost, name: &str, body: &[u8]) -> TtsCatalogEntry {
    let mut e = tiny_entry(&format!("phonon-voice-test-{}", name.to_lowercase()), &[]);
    e.parent = Some("phonon-test".into());
    e.name = name.to_string();
    e.license = Some("Apache-2.0".into());
    e.license_non_commercial = false;
    e.source = Some("hf://gradium/phonon-voices@0ee17205f254b5abc43b266c8a0f215565c352f7".into());
    e.voice = Some(PhononVoiceMeta {
        filename: format!("voices/ID{name}.safetensors"),
        name: name.to_string(),
        voice_id: format!("ID{name}"),
        language: "de".into(),
        locale: "de-de".into(),
        gender: Some("female".into()),
        age: Some("Adult".into()),
        description: format!("Stimme {name}"),
    });
    e.files = vec![catalog::TtsCatalogFile {
        filename: format!("{name}.safetensors"),
        url: format!("{}/voices/{name}", host.base),
        size_bytes: body.len() as u64,
        sha256: Some(sha_hex(body)),
    }];
    e
}

fn parent_fixture() -> (TtsCatalogEntry, [(&'static str, &'static [u8]); 1]) {
    let files: [(&str, &[u8]); 1] = [("config.json", b"{}")];
    (tiny_entry("phonon-test", &files), files)
}

#[tokio::test]
async fn eine_stimme_braucht_das_installierte_modell() {
    let host = fake_host(vec![]).await;
    let h = Harness::new(None, None);
    let (parent, _) = parent_fixture();
    let voice = voice_fixture(&host, "Nele", b"voice-bytes");

    let err = install_voice(&h.env(host.origin()), &voice, &parent)
        .await
        .unwrap_err();

    assert!(err.contains("Zuerst das Modell installieren"), "{err}");
    assert!(host.hits().is_empty());
}

#[tokio::test]
async fn stimmen_werden_geladen_und_in_source_json_eingetragen() {
    let host = fake_host(vec![
        ("/voices/Nele", Reply::Body(b"nele-bytes".to_vec())),
        ("/voices/Anton", Reply::Body(b"anton-bytes!".to_vec())),
    ])
    .await;
    let h = Harness::new(None, None);
    let (parent, files) = parent_fixture();
    write_model_files(&h.dirs.model_dir("phonon-test"), &parent, &files);
    let nele = voice_fixture(&host, "Nele", b"nele-bytes");
    let anton = voice_fixture(&host, "Anton", b"anton-bytes!");

    assert_eq!(
        install_voice(&h.env(host.origin()), &nele, &parent)
            .await
            .unwrap(),
        Outcome::Installed
    );
    let voices = h.dirs.voices_dir("phonon-test");
    assert_eq!(
        fs::read(voices.join("Nele.safetensors")).unwrap(),
        b"nele-bytes"
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&fs::read(voices.join("SOURCE.json")).unwrap()).unwrap();
    assert_eq!(
        doc["source"],
        "hf://gradium/phonon-voices@0ee17205f254b5abc43b266c8a0f215565c352f7"
    );
    assert_eq!(doc["license"], "apache-2.0");
    assert_eq!(doc["voices"].as_array().unwrap().len(), 1);
    assert_eq!(doc["voices"][0]["name"], "Nele");
    assert_eq!(doc["voices"][0]["voice_id"], "IDNele");
    assert_eq!(doc["voices"][0]["filename"], "voices/IDNele.safetensors");
    assert_eq!(doc["voices"][0]["gender"], "female");

    install_voice(&h.env(host.origin()), &anton, &parent)
        .await
        .unwrap();
    let doc: serde_json::Value =
        serde_json::from_slice(&fs::read(voices.join("SOURCE.json")).unwrap()).unwrap();
    let names: Vec<&str> = doc["voices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Nele", "Anton"]);

    // Noch einmal dieselbe Stimme: kein neuer Download, keine doppelte Zeile.
    let before = host.hits().len();
    install_voice(&h.env(host.origin()), &nele, &parent)
        .await
        .unwrap();
    assert_eq!(host.hits().len(), before);
    let doc: serde_json::Value =
        serde_json::from_slice(&fs::read(voices.join("SOURCE.json")).unwrap()).unwrap();
    assert_eq!(doc["voices"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn eine_beschaedigte_stimme_wird_nicht_eingetragen() {
    let host = fake_host(vec![(
        "/voices/Nele",
        Reply::Body(b"falsche-bytes".to_vec()),
    )])
    .await;
    let h = Harness::new(None, None);
    let (parent, files) = parent_fixture();
    write_model_files(&h.dirs.model_dir("phonon-test"), &parent, &files);
    let nele = voice_fixture(&host, "Nele", b"nele-bytes!!"); // andere Pruefsumme/Groesse

    assert!(install_voice(&h.env(host.origin()), &nele, &parent)
        .await
        .is_err());
    let voices = h.dirs.voices_dir("phonon-test");
    assert!(!voices.join("Nele.safetensors").exists());
    assert!(!voices.join("SOURCE.json").exists());
}

// ---- Stimmen gehoeren zum Modell -------------------------------------------------------------------------------------------

/// Modell auf dem Fake-Host samt zwei Stimmen (`/voices/Nele`, `/voices/Anton`).
async fn bundle_host() -> (
    FakeHost,
    TtsCatalogEntry,
    Vec<TtsCatalogEntry>,
    Vec<(&'static str, Vec<u8>)>,
) {
    let (_, bodies) = model_on(&fake_host(vec![]).await, false);
    let mut routes: Vec<(String, Reply)> = routes_for(&bodies);
    routes.push(("/voices/Nele".into(), Reply::Body(b"nele-bytes".to_vec())));
    routes.push((
        "/voices/Anton".into(),
        Reply::Body(b"anton-bytes!".to_vec()),
    ));
    let host = fake_host(
        routes
            .iter()
            .map(|(p, r)| (p.as_str(), r.clone()))
            .collect(),
    )
    .await;
    let (entry, bodies) = model_on(&host, false);
    let voices = vec![
        voice_fixture(&host, "Nele", b"nele-bytes"),
        voice_fixture(&host, "Anton", b"anton-bytes!"),
    ];
    (host, entry, voices, bodies)
}

fn source_names(dirs: &PhononDirs) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_slice(
        &fs::read(dirs.voices_dir("phonon-test").join("SOURCE.json")).unwrap(),
    )
    .unwrap();
    doc["voices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn die_mitgelieferten_stimmen_kommen_aus_dem_katalog_je_modell() {
    let de = bundled_voices("phonon-40m-de");
    let en = bundled_voices("phonon-31m-en");
    assert_eq!((de.len(), en.len()), (13, 37));
    assert!(de
        .iter()
        .all(|v| v.parent.as_deref() == Some("phonon-40m-de")));
    assert!(bundled_voices("phonon-unbekannt").is_empty());
    assert!(bundled_voices("").is_empty());
}

#[tokio::test]
async fn ein_modell_laedt_seine_stimmen_mit_und_oeffentliche_stimmen_bekommen_nie_ein_token() {
    let (host, mut entry, voices, bodies) = bundle_host().await;
    entry.gated = true; // gesperrtes Modell: das Token gilt nur fuer dessen Dateien
    let h = Harness::new(Some(TOKEN), None);

    let out = install_model_with_voices(&h.env(host.origin()), &entry, &voices)
        .await
        .unwrap();

    assert_eq!(out, Outcome::Installed);
    let dir = h.dirs.model_dir("phonon-test");
    for (name, body) in &bodies {
        assert_eq!(&fs::read(dir.join(name)).unwrap(), body, "{name}");
    }
    let vdir = h.dirs.voices_dir("phonon-test");
    assert_eq!(
        fs::read(vdir.join("Nele.safetensors")).unwrap(),
        b"nele-bytes"
    );
    assert_eq!(
        fs::read(vdir.join("Anton.safetensors")).unwrap(),
        b"anton-bytes!"
    );
    assert_eq!(source_names(&h.dirs), ["Nele", "Anton"]);
    assert!(model_complete(&h.dirs, &entry));
    assert_eq!(voices_present(&h.dirs, "phonon-test", &voices), 2);
    assert!(
        !h.dirs.staging_dir("phonon-test").exists(),
        "Zwischenordner ist weg"
    );
    // Das Token ging nur an die Modelldateien, nie an die Stimmen.
    for hit in host.hits() {
        if hit.path.starts_with("/voices/") {
            assert!(hit.auth.is_none(), "{hit:?}");
        } else {
            assert_eq!(
                hit.auth.as_deref(),
                Some(format!("Bearer {TOKEN}").as_str())
            );
        }
    }
    // Ein Fortschritt ueber Modell UND Stimmen, nie rueckwaerts, am Ende voll.
    let total: u64 = bodies.iter().map(|(_, b)| b.len() as u64).sum::<u64>() + 10 + 12;
    let progress = h.progress.lock().unwrap().clone();
    assert!(progress.iter().all(|(_, t)| *t == total), "{progress:?}");
    assert!(
        progress.windows(2).all(|w| w[0].0 <= w[1].0),
        "{progress:?}"
    );
    assert_eq!(progress.last().copied(), Some((total, total)));
    assert_eq!(
        h.verified.load(Ordering::SeqCst),
        5,
        "jede Datei, auch jede Stimme, wird geprueft"
    );
}

#[tokio::test]
async fn eine_beschaedigte_stimme_installiert_das_modell_nicht_und_die_marke_fehlt() {
    let (host, entry, mut voices, _) = bundle_host().await;
    // Anton kommt mit anderen Bytes an, als der Katalog verspricht.
    voices[1].files[0].sha256 = Some(sha_hex(b"ganz-anderes"));
    let h = Harness::new(None, None);

    let err = install_model_with_voices(&h.env(host.origin()), &entry, &voices)
        .await
        .unwrap_err();

    assert!(err.contains("Anton"), "die Meldung nennt die Stimme: {err}");
    assert!(
        !h.dirs.model_dir("phonon-test").exists(),
        "nichts am Zielort"
    );
    assert!(!model_complete(&h.dirs, &entry));
    let staging = h.dirs.staging_dir("phonon-test");
    assert!(
        !staging.join(MARKER_FILE).exists(),
        "die Marke kommt zuletzt"
    );
    assert!(!staging.join("voices").join("Anton.safetensors").exists());
}

#[tokio::test]
async fn abbruch_bei_den_stimmen_und_wiederaufnahme_laedt_fertiges_nicht_neu() {
    let (host, entry, voices, _) = bundle_host().await;
    // Abbruch nach vier geprueften Dateien: drei Modelldateien und die erste Stimme.
    let h = Harness::new(None, Some(4));
    let out = install_model_with_voices(&h.env(host.origin()), &entry, &voices)
        .await
        .unwrap();
    assert_eq!(out, Outcome::Cancelled);
    assert!(!h.dirs.model_dir("phonon-test").exists());
    let staging = h.dirs.staging_dir("phonon-test");
    assert!(staging.join("voices").join("Nele.safetensors").is_file());
    assert!(!staging.join(MARKER_FILE).exists());

    let cancel = CancellationToken::new();
    let progress_cb = |_: HttpDownloadEvent<'_>| {};
    let token = || -> Option<Secret> { None };
    let lock = Mutex::new(());
    let env = InstallEnv {
        dirs: &h.dirs,
        platform: Some("windows-x64"),
        cancel: &cancel,
        emit: &progress_cb,
        event_id: "phonon-test",
        token: &token,
        auth_origin: host.origin(),
        voices_lock: &lock,
    };
    assert_eq!(
        install_model_with_voices(&env, &entry, &voices)
            .await
            .unwrap(),
        Outcome::Installed
    );
    assert_eq!(
        host.count("/files/model.q8.gguf"),
        1,
        "Modell nicht erneut geladen"
    );
    assert_eq!(
        host.count("/voices/Nele"),
        1,
        "fertige Stimme nicht erneut geladen"
    );
    assert_eq!(host.count("/voices/Anton"), 1);
    assert_eq!(source_names(&h.dirs), ["Nele", "Anton"]);
}

#[tokio::test]
async fn ein_installiertes_modell_ohne_stimmen_wird_ergaenzt_ohne_das_modell_neu_zu_laden() {
    let (host, entry, voices, _) = bundle_host().await;
    let h = Harness::new(None, None);
    // Stand von Patrick: Modell ohne Stimmen (frueherer Installer).
    install_model_with_voices(&h.env(host.origin()), &entry, &[])
        .await
        .unwrap();
    let dir = h.dirs.model_dir("phonon-test");
    assert!(!dir.join("voices").exists());
    assert_eq!(voices_present(&h.dirs, "phonon-test", &voices), 0);
    let marker_before = fs::read(dir.join(MARKER_FILE)).unwrap();
    let model_hits = host.hits().len();
    assert_eq!(model_hits, 3);

    // Nochmal installieren: es kommen nur die Stimmen dazu.
    let out = install_model_with_voices(&h.env(host.origin()), &entry, &voices)
        .await
        .unwrap();

    assert_eq!(out, Outcome::Installed);
    assert_eq!(voices_present(&h.dirs, "phonon-test", &voices), 2);
    assert_eq!(source_names(&h.dirs), ["Nele", "Anton"]);
    assert_eq!(
        host.count("/files/model.q8.gguf"),
        1,
        "Modell nicht neu geladen"
    );
    assert_eq!(host.count("/files/config.json"), 1);
    assert_eq!(host.hits().len(), model_hits + 2, "nur die zwei Stimmen");
    assert_eq!(
        fs::read(dir.join(MARKER_FILE)).unwrap(),
        marker_before,
        "die Marke des Modells bleibt unberuehrt"
    );
    assert!(model_complete(&h.dirs, &entry));

    // Vollstaendig: ein weiterer Aufruf macht nichts mehr.
    let before = host.hits().len();
    install_model_with_voices(&h.env(host.origin()), &entry, &voices)
        .await
        .unwrap();
    assert_eq!(host.hits().len(), before);
}

#[tokio::test]
async fn einzeln_installierte_stimmen_bleiben_gueltig_und_nur_der_rest_wird_nachgeladen() {
    let (host, entry, voices, _) = bundle_host().await;
    let h = Harness::new(None, None);
    install_model_with_voices(&h.env(host.origin()), &entry, &[])
        .await
        .unwrap();
    // Aeltere Einzelinstallation: Nele ueber den alten Weg, mit eigenem Eintrag.
    install_voice(&h.env(host.origin()), &voices[0], &entry)
        .await
        .unwrap();
    assert_eq!(voices_present(&h.dirs, "phonon-test", &voices), 1);

    install_model_with_voices(&h.env(host.origin()), &entry, &voices)
        .await
        .unwrap();

    assert_eq!(host.count("/voices/Nele"), 1, "Nele nicht erneut geladen");
    assert_eq!(host.count("/voices/Anton"), 1);
    assert_eq!(voices_present(&h.dirs, "phonon-test", &voices), 2);
    assert_eq!(source_names(&h.dirs), ["Nele", "Anton"]);
}

#[tokio::test]
async fn fehlt_nur_der_eintrag_in_source_json_wird_er_ohne_neuen_download_ergaenzt() {
    let (host, entry, voices, _) = bundle_host().await;
    let h = Harness::new(None, None);
    install_model_with_voices(&h.env(host.origin()), &entry, &voices)
        .await
        .unwrap();
    fs::remove_file(h.dirs.voices_dir("phonon-test").join("SOURCE.json")).unwrap();
    assert_eq!(
        voices_present(&h.dirs, "phonon-test", &voices),
        0,
        "ohne Eintrag unvollstaendig"
    );
    let before = host.hits().len();

    install_model_with_voices(&h.env(host.origin()), &entry, &voices)
        .await
        .unwrap();

    assert_eq!(
        host.hits().len(),
        before,
        "die Dateien stimmen, nichts wird geladen"
    );
    assert_eq!(voices_present(&h.dirs, "phonon-test", &voices), 2);
}

#[tokio::test]
async fn eine_neuinstallation_ueber_ein_unvollstaendiges_modell_mit_alten_stimmen_behaelt_und_ergaenzt(
) {
    let (host, entry, voices, _) = bundle_host().await;
    let h = Harness::new(None, None);
    // Modellordner ohne Marke (unterbrochen) mit schon liegender Stimme Nele samt Eintrag.
    let dir = h.dirs.model_dir("phonon-test");
    fs::create_dir_all(dir.join("voices")).unwrap();
    fs::write(dir.join("voices").join("Nele.safetensors"), b"nele-bytes").unwrap();
    add_voice_row(&dir.join("voices"), &voices[0]).unwrap();
    fs::write(dir.join("config.json"), b"alt").unwrap();

    install_model_with_voices(&h.env(host.origin()), &entry, &voices)
        .await
        .unwrap();

    assert_eq!(host.count("/voices/Nele"), 0, "Nele lag schon da");
    assert_eq!(host.count("/voices/Anton"), 1);
    assert_eq!(fs::read(dir.join("config.json")).unwrap(), b"{\"a\":1}");
    assert_eq!(source_names(&h.dirs), ["Nele", "Anton"]);
    assert!(model_complete(&h.dirs, &entry));
    assert!(!h.dirs.staging_dir("phonon-test").exists());
}

#[test]
fn der_zaehler_der_liste_folgt_den_stimmen_auf_der_platte_und_entfernen_nimmt_sie_mit() {
    let tmp = TempDir::new().unwrap();
    let dirs = PhononDirs::new(tmp.path());
    let lock = Mutex::new(());
    make_model(&dirs, "phonon-31m-en");
    let row = |dirs: &PhononDirs| {
        build_downloads(dirs, None, |_| false)
            .into_iter()
            .find(|r| r.id == "phonon-31m-en")
            .unwrap()
    };
    let r = row(&dirs);
    assert!(r.is_downloaded);
    assert_eq!((r.bundled_voices, r.bundled_voices_present), (37, 0));

    // Drei Stimmen: Datei + Eintrag zaehlt; Datei ohne Eintrag und falsche Groesse zaehlen nicht.
    let voices = bundled_voices("phonon-31m-en");
    for v in &voices[..3] {
        let path = voice_file(&dirs, v).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::File::create(&path).unwrap().set_len(256_136).unwrap();
        add_voice_row(path.parent().unwrap(), v).unwrap();
    }
    assert_eq!(row(&dirs).bundled_voices_present, 3);
    let extra = voice_file(&dirs, &voices[3]).unwrap();
    fs::File::create(&extra).unwrap().set_len(256_136).unwrap();
    assert_eq!(
        row(&dirs).bundled_voices_present,
        3,
        "ohne Eintrag unvollstaendig"
    );
    add_voice_row(extra.parent().unwrap(), &voices[3]).unwrap();
    assert_eq!(row(&dirs).bundled_voices_present, 4);
    fs::File::create(&extra).unwrap().set_len(5).unwrap();
    assert_eq!(
        row(&dirs).bundled_voices_present,
        3,
        "falsche Groesse zaehlt nicht"
    );

    // Das Modell entfernen nimmt den ganzen Ordner mit: Stimmen und SOURCE.json.
    delete(&dirs, None, &lock, "phonon-31m-en").unwrap();
    assert!(!dirs.model_dir("phonon-31m-en").exists());
    assert_eq!(voices_present(&dirs, "phonon-31m-en", &voices), 0);
}

// ---- Entfernen --------------------------------------------------------------------------------------------------------------

#[test]
fn stimmen_entfernen_nimmt_datei_und_zeile_mit_und_raeumt_den_leeren_ordner_auf() {
    let tmp = TempDir::new().unwrap();
    let dirs = PhononDirs::new(tmp.path());
    let lock = Mutex::new(());
    let all = voice_entries();
    let nele = all.iter().find(|e| e.id == "phonon-voice-de-nele").unwrap();
    let anton = all
        .iter()
        .find(|e| e.id == "phonon-voice-de-anton")
        .unwrap();
    for v in [nele, anton] {
        let path = voice_file(&dirs, v).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::File::create(&path).unwrap().set_len(256_136).unwrap();
        add_voice_row(path.parent().unwrap(), v).unwrap();
    }
    let voices = dirs.voices_dir("phonon-40m-de");
    // Das Format der Datei: wie ihn der andere Zweig (list_voices) liest.
    let doc: serde_json::Value =
        serde_json::from_slice(&fs::read(voices.join("SOURCE.json")).unwrap()).unwrap();
    assert_eq!(doc["license"], "apache-2.0");
    assert!(doc["source"]
        .as_str()
        .unwrap()
        .starts_with("hf://gradium/phonon-voices@0ee17205"));
    assert_eq!(
        doc["voices"][0],
        serde_json::to_value(nele.voice.as_ref().unwrap()).unwrap()
    );

    delete(&dirs, Some("windows-x64"), &lock, "phonon-voice-de-nele").unwrap();
    assert!(!voices.join("Nele.safetensors").exists());
    assert!(voices.join("Anton.safetensors").exists());
    let doc: serde_json::Value =
        serde_json::from_slice(&fs::read(voices.join("SOURCE.json")).unwrap()).unwrap();
    assert_eq!(doc["voices"].as_array().unwrap().len(), 1);
    assert_eq!(doc["voices"][0]["name"], "Anton");

    delete(&dirs, Some("windows-x64"), &lock, "phonon-voice-de-anton").unwrap();
    assert!(
        !voices.exists(),
        "ohne Stimmen: weder SOURCE.json noch leerer Ordner"
    );
    // Noch einmal: nicht installiert.
    assert!(delete(&dirs, Some("windows-x64"), &lock, "phonon-voice-de-anton").is_err());
}

#[test]
fn modell_und_programm_entfernen_und_unbekanntes_ablehnen() {
    let tmp = TempDir::new().unwrap();
    let dirs = PhononDirs::new(tmp.path());
    let lock = Mutex::new(());
    // Modell samt Stimmen und liegengebliebenem Zwischenordner.
    let model = dirs.model_dir("phonon-31m-en");
    fs::create_dir_all(model.join("voices")).unwrap();
    fs::write(model.join("voices").join("Everly.safetensors"), b"v").unwrap();
    fs::write(model.join("voices").join("SOURCE.json"), b"{}").unwrap();
    fs::write(model.join("config.json"), b"{}").unwrap();
    fs::create_dir_all(dirs.staging_dir("phonon-31m-en")).unwrap();
    delete(&dirs, None, &lock, "phonon-31m-en").unwrap();
    assert!(!model.exists() && !dirs.staging_dir("phonon-31m-en").exists());
    assert!(
        delete(&dirs, None, &lock, "phonon-31m-en").is_err(),
        "nicht mehr da"
    );

    // Programm.
    let rt = dirs.runtime_dir("windows-x64");
    fs::create_dir_all(&rt).unwrap();
    fs::write(rt.join("ptts.exe"), b"x").unwrap();
    delete(&dirs, Some("windows-x64"), &lock, RUNTIME_ID).unwrap();
    assert!(!rt.exists());
    assert!(delete(&dirs, Some("windows-x64"), &lock, RUNTIME_ID).is_err());
    assert!(delete(&dirs, None, &lock, RUNTIME_ID).is_err());

    // Nichts ausserhalb des Katalogs: kein Pfadbaustein vom Aufrufer.
    let precious = tmp.path().join("wichtig");
    fs::create_dir_all(&precious).unwrap();
    for bad in [
        "../../wichtig",
        "..",
        "",
        "wichtig",
        "phonon-40m-de/../../wichtig",
    ] {
        assert!(
            delete(&dirs, Some("windows-x64"), &lock, bad).is_err(),
            "{bad:?}"
        );
    }
    assert!(precious.exists());
}

#[tokio::test]
async fn unbekannte_kennungen_werden_nicht_installiert() {
    let host = fake_host(vec![]).await;
    let h = Harness::new(None, None);
    for bad in ["", "piper-runtime", "../x", "phonon-voice-de-niemand"] {
        let err = install(&h.env(host.origin()), bad).await.unwrap_err();
        assert!(err.contains("Unbekannter"), "{bad:?}: {err}");
    }
    assert!(host.hits().is_empty());
}

// ---- Echte Downloads (nur von Hand) ------------------------------------------------------------------
//
// `cargo test --lib -- --ignored echt_` laedt die OEFFENTLICHEN Dateien von GitHub und Hugging Face
// (ptts-Archiv, englisches Modell, eine Stimme) in einen Temp-Ordner (TEMP=D:/lv-build/tmp), nie in
// echte App-Daten, und nie die gesperrten Modelle. Es geht kein Token hinaus.

fn real_env<'a>(
    h: &'a Harness,
    token: &'a (dyn Fn() -> Option<Secret> + Send + Sync),
) -> InstallEnv<'a> {
    InstallEnv {
        dirs: &h.dirs,
        platform: current_platform(),
        cancel: &h.cancel,
        emit: &*h.emit,
        event_id: "phonon-real",
        token,
        auth_origin: Origin::huggingface(),
        voices_lock: &h.lock,
    }
}

/// Programm und englisches Modell wirklich laden, einrichten und mit dem Adapter sprechen lassen.
#[tokio::test]
#[ignore = "echter Download (ptts-Archiv von GitHub, Modell und Stimme von Hugging Face), nur von Hand"]
async fn echt_programm_modell_und_stimme_installieren_und_sprechen() {
    use crate::managers::tts::providers::contract::{
        CancelToken, ProviderHealth, SynthesisOptions, TtsProvider,
    };
    use crate::managers::tts::providers::phonon::{PhononConfig, PhononProvider};

    let Some(platform) = current_platform() else {
        return;
    };
    let h = Harness::new(None, None);
    let no_token = || -> Option<Secret> { None };
    let env = real_env(&h, &no_token);

    // 1. Programm.
    assert_eq!(install(&env, RUNTIME_ID).await.unwrap(), Outcome::Installed);
    let exe = h.dirs.runtime_dir(platform).join(exe_name(platform));
    assert!(exe.is_file(), "{}", exe.display());
    assert!(h.dirs.runtime_dir(platform).join("LICENSE-MIT").is_file());
    eprintln!(
        "ptts: {} ({} Byte)",
        exe.display(),
        fs::metadata(&exe).unwrap().len()
    );

    // 2. Oeffentliches Modell (sieben Dateien, SHA-256 bzw. Git-Blob je Datei geprueft) samt EINER
    // englischen Stimme aus dem Bundle (nicht alle 37: der Test soll klein bleiben).
    let model_entry_en = model_entry("phonon-31m-en").unwrap();
    let everly = bundled_voices("phonon-31m-en")
        .into_iter()
        .find(|v| v.name == "Everly")
        .expect("Everly gehoert zum englischen Modell");
    assert_eq!(
        install_model_with_voices(&env, &model_entry_en, std::slice::from_ref(&everly))
            .await
            .unwrap(),
        Outcome::Installed
    );
    assert!(model_complete(
        &h.dirs,
        &model_entry("phonon-31m-en").unwrap()
    ));
    let progress = h.progress.lock().unwrap().clone();
    eprintln!(
        "Fortschrittsmeldungen: {}, zuletzt {:?}",
        progress.len(),
        progress.last()
    );

    // 3. Die Stimme liegt im Modellordner, mit Eintrag in SOURCE.json.
    let voices = h.dirs.voices_dir("phonon-31m-en");
    assert_eq!(
        fs::metadata(voices.join("Everly.safetensors"))
            .unwrap()
            .len(),
        256_136
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&fs::read(voices.join("SOURCE.json")).unwrap()).unwrap();
    assert_eq!(doc["voices"][0]["name"], "Everly");
    eprintln!("SOURCE.json: {doc}");
    // Mit nur einer von 37 Stimmen fehlen die anderen (Zaehler der Oberflaeche).
    let row = build_downloads(&h.dirs, Some(platform), |_| false)
        .into_iter()
        .find(|r| r.id == "phonon-31m-en")
        .unwrap();
    assert_eq!((row.bundled_voices, row.bundled_voices_present), (37, 1));

    // 4. Der Adapter spricht mit dem Installierten (ProcessGuard, RAM-Start-Gate wie im Betrieb).
    let paths = installed_paths(h._tmp.path());
    assert_eq!(
        paths.phonon.exe_path.as_deref(),
        Some(exe.to_string_lossy().as_ref())
    );
    assert!(paths
        .phonon
        .model
        .as_deref()
        .unwrap()
        .ends_with("phonon-31m-en"));
    let provider = PhononProvider::new(PhononConfig {
        exe_path: paths.phonon.exe_path.clone().unwrap(),
        model: paths.phonon.model.clone().unwrap(),
        lang: "en".into(),
        quant: Some("q8".into()),
        ..PhononConfig::default()
    });
    let lic = provider.descriptor().license;
    assert!(lic.verified && !lic.non_commercial, "{lic:?}");
    let health: ProviderHealth = provider.health_check().await;
    assert!(health.available, "{health:?}");
    eprintln!("Version laut ptts: {:?}", health.version);
    let mut clips = Vec::new();
    for voice in [None, Some("Everly".to_string())] {
        let opts = SynthesisOptions {
            language: "en".into(),
            voice: voice.clone(),
            timeout: std::time::Duration::from_secs(120),
            ..Default::default()
        };
        let clip = provider
            .synthesize(
                "Hello from the Local Voice test.",
                &opts,
                &CancelToken::detached(),
            )
            .await
            .unwrap_or_else(|e| panic!("{voice:?}: {e:?}"));
        assert!(clip.duration_ms > 500, "{voice:?}: {} ms", clip.duration_ms);
        eprintln!("Stimme {voice:?}: {} ms Audio", clip.duration_ms);
        clips.push(clip.data);
    }
    // Die installierte Stimme wirkt wirklich: anderes Audio als die Standardstimme.
    assert_ne!(
        clips[0], clips[1],
        "die Stimme Everly aus voices/ wurde nicht verwendet"
    );
}

/// Der Weg mit eigener Weiterleitungsfolge (wie fuer gesperrte Modelle) gegen den echten Hugging-Face-
/// Server: ein oeffentliches LFS-Objekt wird ueber Weiterleitung auf das CDN geladen. Das Token ist
/// hier an einen anderen Ursprung gebunden, geht also nirgends hin; geprueft wird das Folgen.
#[tokio::test]
#[ignore = "echter Download einer oeffentlichen Datei von Hugging Face, nur von Hand"]
async fn echt_weiterleitung_zum_cdn_wird_gefolgt_ohne_token() {
    let nele = voice_entries()
        .into_iter()
        .find(|e| e.id == "phonon-voice-de-nele")
        .unwrap();
    let file = &nele.files[0];
    let tmp = TempDir::new().unwrap();
    let partial = tmp.path().join("Nele.safetensors.partial");
    let other_origin = DownloadAuth::new(
        Secret::new("hf_kein_echtes_token_wird_nie_gesendet").unwrap(),
        Origin::plain_http("127.0.0.1", 9),
    );
    let out = ModelManager::download_http_resumable_authed(
        "phonon-real",
        &file.url,
        &partial,
        Some(file.size_bytes),
        file.sha256.as_deref(),
        &CancellationToken::new(),
        &|_| {},
        Some(&other_origin),
    )
    .await
    .unwrap();
    assert!(matches!(out, HttpDownloadOutcome::Completed));
    assert_eq!(fs::metadata(&partial).unwrap().len(), 256_136);
}
