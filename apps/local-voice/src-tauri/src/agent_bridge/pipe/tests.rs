//! Tests der echten Named Pipe (nur Windows): DACL, Fernzugriff, Squatting, Gegenueber,
//! Ende-zu-Ende mit dem blockierenden Client.

use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::oneshot;
use ulid::Ulid;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::{
    CreateWellKnownSid, GetAce, GetAclInformation, GetKernelObjectSecurity,
    GetSecurityDescriptorDacl, AclSizeInformation, ACL, ACL_SIZE_INFORMATION, PSECURITY_DESCRIPTOR,
    PSID, WinWorldSid,
};

use super::*;
use crate::agent_bridge::client::{Client, ClientError};
use crate::agent_bridge::server::Server;
use crate::agent_bridge::testkit::Fixture;
use crate::managers::integrations::model::GrantMode;
use tokio::net::windows::named_pipe::ServerOptions;

fn unique_name() -> String {
    format!(r"\\.\pipe\lva-test-{}", Ulid::new())
}

fn world_sid() -> OwnedSid {
    // SAFETY: zweistufiger Aufruf mit eigenem Puffer.
    unsafe {
        let mut len = 0u32;
        let _ = CreateWellKnownSid(WinWorldSid, None, None, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        CreateWellKnownSid(
            WinWorldSid,
            None,
            Some(PSID(buf.as_mut_ptr() as *mut c_void)),
            &mut len,
        )
        .unwrap();
        OwnedSid::from_psid(PSID(buf.as_ptr() as *mut c_void)).unwrap()
    }
}

/// Die Eintraege der DACL einer Pipe: (ACE-Typ, SID als Text). Typ 0 = erlaubt, 1 = verboten.
fn dacl_entries(pipe: &impl AsRawHandle) -> Vec<(u8, String)> {
    // SAFETY: Puffer werden nach Groessenabfrage angelegt; Zeiger bleiben im Puffer.
    unsafe {
        let handle = HANDLE(pipe.as_raw_handle());
        let mut needed = 0u32;
        let _ = GetKernelObjectSecurity(handle, 4, None, 0, &mut needed);
        assert!(needed > 0);
        let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
        let psd = PSECURITY_DESCRIPTOR(buf.as_mut_ptr() as *mut c_void);
        GetKernelObjectSecurity(handle, 4, Some(psd), needed, &mut needed).unwrap();
        let mut present = windows::core::BOOL::default();
        let mut defaulted = windows::core::BOOL::default();
        let mut pacl: *mut ACL = std::ptr::null_mut();
        GetSecurityDescriptorDacl(psd, &mut present, &mut pacl, &mut defaulted).unwrap();
        assert!(present.as_bool(), "die Pipe hat eine DACL");
        let mut info = ACL_SIZE_INFORMATION::default();
        GetAclInformation(
            pacl,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
        .unwrap();
        let mut out = Vec::new();
        for i in 0..info.AceCount {
            let mut ace: *mut c_void = std::ptr::null_mut();
            GetAce(pacl, i, &mut ace).unwrap();
            let kind = *(ace as *const u8);
            // ACE: Kopf (4 Bytes), Maske (4 Bytes), danach die SID.
            let sid = PSID((ace as *mut u8).add(8) as *mut c_void);
            out.push((kind, sid_to_string(sid)));
        }
        out
    }
}

// --- Namen ------------------------------------------------------------------------

#[test]
fn the_default_name_carries_a_hash_of_the_user_sid() {
    let name = effective_pipe_name_for_test();
    let key = user_key().unwrap();
    assert_eq!(key.len(), 16);
    assert!(key.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(name, format!(r"\\.\pipe\local-voice-ai-agent-{key}"));
    assert_eq!(user_key().unwrap(), key, "stabil");
}

fn effective_pipe_name_for_test() -> String {
    resolve_pipe_name(false, None, &user_key().unwrap()).unwrap()
}

#[test]
fn a_custom_pipe_name_is_only_honoured_in_the_sandbox_and_only_with_safe_characters() {
    assert_eq!(
        resolve_pipe_name(true, Some("lva-test.1"), "abc").unwrap(),
        r"\\.\pipe\lva-test.1"
    );
    assert!(resolve_pipe_name(false, Some("lva-test"), "abc").is_err(), "ausserhalb der Sandbox nie");
    for bad in ["a b", "a\\b", "a/b", "..\\x", "a:b", &"x".repeat(65)] {
        assert!(resolve_pipe_name(true, Some(bad), "abc").is_err(), "{bad}");
    }
    // Leer oder nur Leerzeichen: der Standard.
    assert_eq!(
        resolve_pipe_name(false, Some("  "), "abc").unwrap(),
        r"\\.\pipe\local-voice-ai-agent-abc"
    );
}

#[test]
fn sids_are_formatted_like_windows_does() {
    let sid = current_user_sid().unwrap().to_sid_string();
    assert!(sid.starts_with("S-1-5-21-") || sid.starts_with("S-1-5-"), "{sid}");
    assert_eq!(world_sid().to_sid_string(), "S-1-1-0");
}

// --- DACL -------------------------------------------------------------------------

#[tokio::test]
async fn the_pipe_dacl_allows_only_the_current_user_and_denies_network_logons() {
    let own = current_user_sid().unwrap();
    let mut sec = PipeSecurity::only(current_user_sid().unwrap()).unwrap();
    let pipe = create_instance(&unique_name(), &mut sec, true).unwrap();
    let entries = dacl_entries(&pipe);
    assert_eq!(
        entries,
        vec![(1u8, "S-1-5-2".to_string()), (0u8, own.to_sid_string())],
        "erst das Verbot fuer Netzwerk-Anmeldungen, dann nur der aktuelle Benutzer"
    );
    // Insbesondere nichts fuer Jeden, Benutzer, authentifizierte Benutzer, SYSTEM, Administratoren.
    for (kind, sid) in &entries {
        if *kind == 0 {
            for broad in ["S-1-1-0", "S-1-5-32-545", "S-1-5-11", "S-1-5-18", "S-1-5-32-544"] {
                assert_ne!(sid, broad);
            }
        }
    }
}

#[tokio::test]
async fn further_instances_get_the_same_protection() {
    let mut sec = PipeSecurity::only(current_user_sid().unwrap()).unwrap();
    let name = unique_name();
    let first = create_instance(&name, &mut sec, true).unwrap();
    let second = create_instance(&name, &mut sec, false).unwrap();
    assert_eq!(dacl_entries(&first), dacl_entries(&second));
}

#[tokio::test]
async fn a_second_first_instance_is_refused() {
    let mut sec = PipeSecurity::only(current_user_sid().unwrap()).unwrap();
    let name = unique_name();
    let _first = create_instance(&name, &mut sec, true).unwrap();
    // Wer den Namen schon belegt hat (auch wir selbst), blockiert eine zweite „erste“ Instanz.
    let again = create_instance(&name, &mut sec, true);
    assert!(again.is_err(), "der Name ist belegt");
    assert_eq!(again.err().unwrap().kind(), io::ErrorKind::PermissionDenied);
}

#[tokio::test]
async fn a_squatted_name_stops_the_server_loudly() {
    let name = unique_name();
    let mut sec = PipeSecurity::only(current_user_sid().unwrap()).unwrap();
    let _squatter = create_instance(&name, &mut sec, true).unwrap();
    let f = Fixture::new();
    let server = Server::new(f.bridge.clone());
    let (tx, rx) = oneshot::channel();
    // Ohne die Sperre der ersten Instanz wuerde `serve` einfach weiterlaufen: dann endet der
    // Test mit einem Fehler statt zu haengen.
    let result = tokio::time::timeout(Duration::from_secs(5), serve(server, &name, Some(tx)))
        .await
        .expect("serve meldet den belegten Namen und kehrt zurueck");
    assert!(result.is_err());
    assert!(rx.await.unwrap().is_err(), "ready meldet den Fehler");
}

#[tokio::test]
async fn remote_clients_are_rejected() {
    // Kontrolle: dieselbe Pipe OHNE Sperre fuer Fernzugriffe ist ueber den UNC-Weg erreichbar.
    let control_name = unique_name();
    let control_leaf = control_name.trim_start_matches(r"\\.\pipe\").to_string();
    let _control = ServerOptions::new()
        .reject_remote_clients(false)
        .max_instances(4)
        .create(&control_name)
        .unwrap();
    let unc = |leaf: &str| format!(r"\\127.0.0.1\pipe\{leaf}");
    if OpenOptions::new().read(true).write(true).open(unc(&control_leaf)).is_err() {
        eprintln!("UNC-Weg ueber 127.0.0.1 ist auf diesem Rechner nicht verfuegbar: Test uebersprungen");
        return;
    }
    // Die gesicherte Pipe der Bruecke weist denselben Weg ab.
    let name = unique_name();
    let leaf = name.trim_start_matches(r"\\.\pipe\").to_string();
    let mut sec = PipeSecurity::only(current_user_sid().unwrap()).unwrap();
    let _pipe = create_instance(&name, &mut sec, true).unwrap();
    let remote = OpenOptions::new().read(true).write(true).open(unc(&leaf));
    assert!(remote.is_err(), "Fernzugriff muss scheitern, ging aber durch");
    // Lokal geht sie auf.
    assert!(OpenOptions::new().read(true).write(true).open(&name).is_ok());
}

// --- Gegenueber -------------------------------------------------------------------

#[test]
fn a_process_of_the_current_user_passes_and_another_sid_does_not() {
    let own = current_user_sid().unwrap();
    assert!(pid_belongs_to(std::process::id(), &own).unwrap());
    assert!(!pid_belongs_to(std::process::id(), &world_sid()).unwrap());
    // Ein Prozess, den es nicht gibt, ist nicht pruefbar: Fehler (gilt als Nein).
    assert!(pid_belongs_to(0x7FFF_FFF0, &own).is_err());
}

// --- Ende zu Ende -----------------------------------------------------------------

async fn start_server(f: &Fixture, name: &str) -> (Arc<Server>, tokio::task::JoinHandle<io::Result<()>>) {
    let server = Server::new(f.bridge.clone());
    let (tx, rx) = oneshot::channel();
    let s2 = server.clone();
    let n = name.to_string();
    let handle = tokio::spawn(async move { serve(s2, &n, Some(tx)).await });
    rx.await.unwrap().expect("Pipe angelegt");
    (server, handle)
}

/// Fuehrt `work` mit einem verbundenen Client auf einem Arbeitsthread aus.
async fn with_client<T: Send + 'static>(
    name: &str,
    work: impl FnOnce(&mut Client<std::io::BufReader<std::fs::File>, std::fs::File>) -> T + Send + 'static,
) -> T {
    let name = name.to_string();
    tokio::task::spawn_blocking(move || {
        let mut c = Client::connect(&name).expect("verbunden");
        work(&mut c)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn end_to_end_status_login_list_and_call_over_the_real_pipe() {
    let f = Fixture::new();
    f.grant("create_meeting", GrantMode::Allow);
    f.grant("transcribe_file", GrantMode::Off);
    let name = unique_name();
    let (_server, _h) = start_server(&f, &name).await;
    let token = f.token.clone();
    let out = with_client(&name, move |c| {
        let anon = c.request("status", json!({})).unwrap();
        let hello = c.hello(Some(&token)).unwrap();
        let tools = c.request("tools/list", json!({})).unwrap();
        let call = c
            .request("tools/call", json!({"name": "create_meeting", "arguments": {"title": "T"}}))
            .unwrap();
        let off = c.request("tools/call", json!({"name": "transcribe_file", "arguments": {}}));
        (anon, hello, tools, call, off)
    })
    .await;
    assert_eq!(out.0["authenticated"], json!(false));
    assert_eq!(out.1["authenticated"], json!(true));
    let names: Vec<&str> = out.2["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"create_meeting") && !names.contains(&"transcribe_file"));
    assert_eq!(out.3["status"], json!("done"));
    match out.4 {
        Err(ClientError::Remote { code, .. }) => assert_eq!(code, "tool_off"),
        other => panic!("{other:?}"),
    }
    assert_eq!(f.calls.len(), 1);
    // Audit: erlaubt (ok) und abgelehnt (denied).
    let outcomes: Vec<String> = f.audit().iter().map(|r| r.outcome.clone()).collect();
    assert_eq!(outcomes, vec!["ok", "denied"]);
}

#[tokio::test]
async fn an_invalid_token_over_the_real_pipe_is_refused_and_audited() {
    let f = Fixture::new();
    let name = unique_name();
    let (_server, _h) = start_server(&f, &name).await;
    let err = with_client(&name, |c| c.hello(Some(&crate::agent_bridge::clients::generate_token()))).await;
    assert!(matches!(&err, Err(ClientError::Remote { code, .. }) if code == "token_invalid"), "{err:?}");
    let rows = f.audit();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "denied");
}

#[tokio::test]
async fn many_clients_at_once_all_get_served() {
    let f = Fixture::new();
    let name = unique_name();
    let (_server, _h) = start_server(&f, &name).await;
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let (n, t) = (name.clone(), f.token.clone());
        tasks.push(tokio::task::spawn_blocking(move || {
            let mut c = Client::connect(&n).unwrap();
            c.hello(Some(&t)).unwrap();
            c.request("status", json!({})).unwrap()
        }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap()["authenticated"], json!(true));
    }
}

#[tokio::test]
async fn a_peer_of_another_sid_is_cut_off_and_audited() {
    let f = Fixture::new();
    let name = unique_name();
    let server = Server::new(f.bridge.clone());
    let (tx, rx) = oneshot::channel();
    let (s2, n2) = (server.clone(), name.clone());
    // Der Server erwartet einen anderen Benutzer als den, der sich verbindet.
    tokio::spawn(async move { serve_with(s2, &n2, Some(tx), world_sid()).await });
    rx.await.unwrap().unwrap();
    let result = with_client(&name, |c| c.request("status", json!({}))).await;
    assert!(result.is_err(), "die Verbindung wird abgewiesen: {result:?}");
    // Das Audit wird im Hintergrund geschrieben.
    for _ in 0..100 {
        if !f.audit().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let rows = f.audit();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "denied");
    assert!(rows[0].detail_json.as_deref().unwrap().contains("peer_not_current_user"));
}

#[tokio::test]
async fn the_client_refuses_a_server_of_another_user() {
    let f = Fixture::new();
    let name = unique_name();
    let (_server, _h) = start_server(&f, &name).await;
    let file = OpenOptions::new().read(true).write(true).open(&name).unwrap();
    // Mit dem eigenen Benutzer als Erwartung ist alles in Ordnung ...
    verify_server(&file).unwrap();
    // ... mit einem anderen nicht.
    match verify_server_for(&file, &world_sid()) {
        Err(ClientError::UntrustedServer(m)) => assert!(m.contains("anderen Benutzer"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn no_app_means_not_running() {
    let name = unique_name();
    let r = tokio::task::spawn_blocking(move || Client::connect(&name)).await.unwrap();
    assert!(matches!(r, Err(ClientError::NotRunning(_))), "{:?}", r.err());
}

#[tokio::test]
async fn shutting_the_server_down_frees_the_name() {
    let f = Fixture::new();
    let name = unique_name();
    let (server, handle) = start_server(&f, &name).await;
    server.shutdown();
    tokio::time::timeout(Duration::from_secs(5), handle).await.unwrap().unwrap().unwrap();
    // Danach laesst sich der Name wieder als erste Instanz anlegen (kein Rest bleibt haengen).
    let mut sec = PipeSecurity::only(current_user_sid().unwrap()).unwrap();
    let mut ok = false;
    for _ in 0..50 {
        if create_instance(&name, &mut sec, true).is_ok() {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "der Name wird frei");
}
