//! Die Named Pipe der Agentenbruecke (A7), nur Windows. Auf anderen Systemen gibt es sie
//! (noch) nicht: `serve` und `connect` melden `Unsupported`.
//!
//! Sicherheitsmerkmale (Review QG4), jedes mit Test in `pipe/tests.rs`:
//! - **Nur der aktuelle Benutzer**: die Pipe wird mit einer eigenen DACL angelegt: erlaubt ist
//!   ausschliesslich die SID des Benutzers, unter dem die App laeuft (Lesen/Schreiben); ein
//!   ausdrueckliches Verbot fuer Netzwerk-Anmeldungen (`NU`) steht davor. Es gibt keinen Eintrag
//!   fuer `Everyone`, `Users`, `Authenticated Users`, `SYSTEM` oder `Administrators`.
//! - **Keine Fernzugriffe**: `PIPE_REJECT_REMOTE_CLIENTS` (SMB-Zugriffe von anderen
//!   Rechnern UND ueber `\\localhost\pipe\...` werden abgewiesen).
//! - **Pruefung des Gegenuebers je Verbindung**: ueber `GetNamedPipeClientProcessId` wird die
//!   Benutzer-SID des Prozesses gelesen und mit der eigenen verglichen; ist sie anders oder
//!   nicht lesbar, wird die Verbindung geschlossen und im Audit vermerkt (Verteidigung in der
//!   Tiefe neben der DACL). Grenze: ein Gegenueber mit erhoehten Rechten (Administrator-Prozess
//!   desselben Benutzers) laesst sich von einer App ohne erhoehte Rechte nicht pruefen und wird
//!   deshalb abgewiesen.
//! - **Kein Squatting**: die erste Instanz wird mit `FILE_FLAG_FIRST_PIPE_INSTANCE` angelegt;
//!   belegt ein anderer Prozess den Namen schon, scheitert der Start laut statt still. Der Name
//!   enthaelt einen Hash der Benutzer-SID, andere Benutzer weichen so aus. Die Gegenstelle
//!   (`client.rs`) prueft umgekehrt die Benutzer-SID des Server-Prozesses.
//! - **Keine Identitaet verleihen**: die Gegenstelle oeffnet die Pipe mit
//!   `SECURITY_ANONYMOUS`; der Server kann sich nie als der Client ausgeben.
//! - Hoechstens 16 Instanzen (Betriebssystem-Grenze), im Server hoechstens 8 Verbindungen.

use std::io;

#[cfg(windows)]
pub use win::*;

#[cfg(not(windows))]
pub use other::*;

/// Umgebungsvariable: eigener Pipe-Name fuer Sandbox-Laeufe (nur wirksam, wenn `LVA_MEETINGS_DIR`
/// gesetzt ist, damit sie nie die Pipe einer produktiven App verdraengt).
pub const PIPE_ENV: &str = "LVA_AGENT_PIPE";
const PIPE_PREFIX: &str = r"\\.\pipe\";
const DEFAULT_PIPE_STEM: &str = "local-voice-ai-agent-";

/// Gueltiger frei waehlbarer Namensteil: Buchstaben, Ziffern, `-`, `_`, `.`, 1 bis 64 Zeichen.
pub fn valid_pipe_suffix(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// Wahl des Pipe-Namens (rein): Standard `\\.\pipe\local-voice-ai-agent-<Benutzerkennung>`;
/// ein Sandbox-Name nur mit `sandbox == true` und gueltiger Schreibweise.
pub fn resolve_pipe_name(
    sandbox: bool,
    override_name: Option<&str>,
    user_key: &str,
) -> Result<String, String> {
    if let Some(name) = override_name.map(str::trim).filter(|n| !n.is_empty()) {
        if !sandbox {
            return Err(format!(
                "{PIPE_ENV} gilt nur zusammen mit {} (Sandbox).",
                crate::managers::meetings::MEETINGS_DIR_ENV
            ));
        }
        if !valid_pipe_suffix(name) {
            return Err(format!("{PIPE_ENV} enthält ungültige Zeichen."));
        }
        return Ok(format!("{PIPE_PREFIX}{name}"));
    }
    Ok(format!("{PIPE_PREFIX}{DEFAULT_PIPE_STEM}{user_key}"))
}

/// Der Pipe-Name dieser Sitzung (Umgebung und Benutzer-SID).
pub fn effective_pipe_name() -> io::Result<String> {
    let sandbox = std::env::var(crate::managers::meetings::MEETINGS_DIR_ENV)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let override_name = std::env::var(PIPE_ENV).ok();
    let key = user_key()?;
    resolve_pipe_name(sandbox, override_name.as_deref(), &key)
        .map_err(|m| io::Error::new(io::ErrorKind::InvalidInput, m))
}

#[cfg(not(windows))]
mod other {
    use std::io;
    use std::sync::Arc;

    use tokio::sync::oneshot;

    use super::super::server::Server;

    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Die Agentenbrücke gibt es nur unter Windows.",
        ))
    }

    pub fn user_key() -> io::Result<String> {
        unsupported()
    }

    pub async fn serve(
        _server: Arc<Server>,
        _name: &str,
        ready: Option<oneshot::Sender<io::Result<()>>>,
    ) -> io::Result<()> {
        if let Some(r) = ready {
            let _ = r.send(Err(io::Error::new(io::ErrorKind::Unsupported, "nur Windows")));
        }
        unsupported()
    }

    pub fn open_client(_name: &str) -> Result<std::fs::File, super::super::client::ClientError> {
        Err(super::super::client::ClientError::NotRunning(
            "Die Agentenbrücke gibt es nur unter Windows.".to_string(),
        ))
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::mem::size_of;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use serde_json::json;
    use sha2::{Digest, Sha256};
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
    use tokio::sync::oneshot;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        AddAccessAllowedAce, AddAccessDeniedAce, CreateWellKnownSid, EqualSid, GetLengthSid,
        GetSidIdentifierAuthority, GetSidSubAuthority, GetSidSubAuthorityCount,
        GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor,
        SetSecurityDescriptorDacl, ACE_REVISION, ACL, ACL_REVISION, PSECURITY_DESCRIPTOR, PSID,
        SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER, TokenUser,
        WinNetworkSid,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    use super::super::client::ClientError;
    use super::super::server::Server;

    /// `FILE_GENERIC_READ | FILE_GENERIC_WRITE` (enthaelt `FILE_CREATE_PIPE_INSTANCE`).
    const MASK_READ_WRITE: u32 = 0x0012_019F;
    /// `FILE_ALL_ACCESS` (fuer das ausdrueckliche Verbot).
    const MASK_ALL: u32 = 0x001F_01FF;
    /// Hoechstzahl Pipe-Instanzen.
    const MAX_INSTANCES: usize = 16;
    /// `SECURITY_SQOS_PRESENT` ohne Stufe = `SECURITY_ANONYMOUS`.
    const SECURITY_SQOS_PRESENT: u32 = 0x0010_0000;
    const ERROR_PIPE_BUSY: i32 = 231;
    const ERROR_FILE_NOT_FOUND: i32 = 2;

    extern "system" {
        fn GetNamedPipeClientProcessId(pipe: *mut c_void, client_process_id: *mut u32) -> i32;
        fn GetNamedPipeServerProcessId(pipe: *mut c_void, server_process_id: *mut u32) -> i32;
    }

    fn win_err(e: windows::core::Error) -> io::Error {
        io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
    }

    // --- Handles und SIDs ----------------------------------------------------------

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: das Handle gehoert uns und wird genau einmal geschlossen.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// Eine SID als eigener, ausgerichteter Speicher.
    pub struct OwnedSid {
        buf: Vec<u64>,
        len: usize,
    }

    impl OwnedSid {
        pub fn from_psid(p: PSID) -> io::Result<Self> {
            // SAFETY: `p` zeigt auf eine gueltige SID (vom Betriebssystem geliefert).
            unsafe {
                let len = GetLengthSid(p) as usize;
                if len == 0 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "leere SID"));
                }
                let mut buf = vec![0u64; len.div_ceil(8)];
                std::ptr::copy_nonoverlapping(p.0 as *const u8, buf.as_mut_ptr() as *mut u8, len);
                Ok(Self { buf, len })
            }
        }

        pub fn psid(&self) -> PSID {
            PSID(self.buf.as_ptr() as *mut c_void)
        }

        pub fn len(&self) -> usize {
            self.len
        }

        pub fn is_empty(&self) -> bool {
            self.len == 0
        }

        /// `S-1-5-21-...`
        pub fn to_sid_string(&self) -> String {
            sid_to_string(self.psid())
        }

        pub fn equals(&self, other: &OwnedSid) -> bool {
            // SAFETY: beide SIDs sind gueltig und leben lange genug.
            unsafe { EqualSid(self.psid(), other.psid()).is_ok() }
        }
    }

    /// Kanonische Textform einer SID (`S-<Revision>-<Behoerde>-<Teil>...`).
    pub fn sid_to_string(psid: PSID) -> String {
        // SAFETY: `psid` ist eine gueltige SID; die Zeiger kommen von den Windows-Funktionen.
        unsafe {
            let revision = *(psid.0 as *const u8);
            let auth = (*GetSidIdentifierAuthority(psid)).Value;
            let count = *GetSidSubAuthorityCount(psid);
            let authority = auth.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b));
            let mut out = format!("S-{revision}-{authority}");
            for i in 0..u32::from(count) {
                out.push_str(&format!("-{}", *GetSidSubAuthority(psid, i)));
            }
            out
        }
    }

    fn token_user_sid(token: HANDLE) -> io::Result<OwnedSid> {
        // SAFETY: zweistufiger Aufruf mit selbst angelegtem, ausgerichtetem Puffer.
        unsafe {
            let mut len = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
            if len == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];
            GetTokenInformation(
                token,
                TokenUser,
                Some(buf.as_mut_ptr() as *mut c_void),
                len,
                &mut len,
            )
            .map_err(win_err)?;
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            OwnedSid::from_psid(user.User.Sid)
        }
    }

    /// Benutzer-SID dieses Prozesses.
    pub fn current_user_sid() -> io::Result<OwnedSid> {
        // SAFETY: `GetCurrentProcess` liefert ein Pseudo-Handle; das Token wird geschlossen.
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(win_err)?;
            let token = OwnedHandle(token);
            token_user_sid(token.0)
        }
    }

    /// Benutzer-SID eines fremden Prozesses (`Err`, wenn er sich nicht oeffnen laesst).
    pub fn process_user_sid(pid: u32) -> io::Result<OwnedSid> {
        // SAFETY: Handles werden geschlossen; nur Abfragerechte.
        unsafe {
            let process = OwnedHandle(
                OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map_err(win_err)?,
            );
            let mut token = HANDLE::default();
            OpenProcessToken(process.0, TOKEN_QUERY, &mut token).map_err(win_err)?;
            let token = OwnedHandle(token);
            token_user_sid(token.0)
        }
    }

    /// Kurzer, stabiler Teil des Pipe-Namens aus der Benutzer-SID (16 Hex-Zeichen).
    pub fn user_key() -> io::Result<String> {
        let sid = current_user_sid()?.to_sid_string();
        Ok(Sha256::digest(sid.as_bytes())
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect())
    }

    // --- DACL ------------------------------------------------------------------------

    /// Sicherheitsbeschreibung der Pipe: Verbot fuer Netzwerk-Anmeldungen, Zugriff nur fuer eine SID.
    /// Haelt alle Speicherbereiche, auf die `SECURITY_ATTRIBUTES` zeigt.
    pub struct PipeSecurity {
        _acl: Vec<u64>,
        _descriptor: Vec<u64>,
        _user: OwnedSid,
        _network: OwnedSid,
        attrs: SECURITY_ATTRIBUTES,
    }

    // SAFETY: die Zeiger in `attrs` zeigen auf die eigenen Vec-Puffer (stabile Heap-Adressen)
    // und werden nur waehrend `create_*` gelesen.
    unsafe impl Send for PipeSecurity {}

    impl PipeSecurity {
        /// Nur diese SID darf zugreifen.
        pub fn only(user: OwnedSid) -> io::Result<Self> {
            // SAFETY: Puffer werden vor dem Aufruf in ausreichender Groesse und Ausrichtung
            // angelegt; die Funktionen schreiben nur hinein.
            unsafe {
                let mut net_len = 0u32;
                let _ = CreateWellKnownSid(WinNetworkSid, None, None, &mut net_len);
                if net_len == 0 {
                    return Err(io::Error::last_os_error());
                }
                let mut net_buf = vec![0u64; (net_len as usize).div_ceil(8)];
                CreateWellKnownSid(
                    WinNetworkSid,
                    None,
                    Some(PSID(net_buf.as_mut_ptr() as *mut c_void)),
                    &mut net_len,
                )
                .map_err(win_err)?;
                let network = OwnedSid {
                    buf: net_buf,
                    len: net_len as usize,
                };

                // Groesse: ACL-Kopf + je ACE (Kopf+Maske+SID, ohne das eingerechnete SID-Feld).
                let ace_overhead = size_of::<windows::Win32::Security::ACCESS_ALLOWED_ACE>() - size_of::<u32>();
                let acl_len = (size_of::<ACL>() + 2 * ace_overhead + user.len() + network.len() + 7) & !7;
                let mut acl = vec![0u64; acl_len / 8];
                let pacl = acl.as_mut_ptr() as *mut ACL;
                InitializeAcl(pacl, acl_len as u32, ACL_REVISION).map_err(win_err)?;
                // Verbote zuerst, dann die Erlaubnis.
                AddAccessDeniedAce(pacl, ACE_REVISION(ACL_REVISION.0), MASK_ALL, network.psid())
                    .map_err(win_err)?;
                AddAccessAllowedAce(pacl, ACE_REVISION(ACL_REVISION.0), MASK_READ_WRITE, user.psid())
                    .map_err(win_err)?;

                let mut descriptor = vec![0u64; size_of::<SECURITY_DESCRIPTOR>().div_ceil(8)];
                let psd = PSECURITY_DESCRIPTOR(descriptor.as_mut_ptr() as *mut c_void);
                InitializeSecurityDescriptor(psd, 1).map_err(win_err)?;
                SetSecurityDescriptorDacl(psd, true, Some(pacl as *const ACL), false)
                    .map_err(win_err)?;

                let attrs = SECURITY_ATTRIBUTES {
                    nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: psd.0,
                    bInheritHandle: false.into(),
                };
                Ok(Self {
                    _acl: acl,
                    _descriptor: descriptor,
                    _user: user,
                    _network: network,
                    attrs,
                })
            }
        }

        fn attrs_ptr(&mut self) -> *mut c_void {
            &mut self.attrs as *mut SECURITY_ATTRIBUTES as *mut c_void
        }
    }

    /// Legt eine Pipe-Instanz an. `first`: erste Instanz des Namens (scheitert, wenn er schon belegt ist).
    pub fn create_instance(
        name: &str,
        sec: &mut PipeSecurity,
        first: bool,
    ) -> io::Result<NamedPipeServer> {
        let mut opts = ServerOptions::new();
        opts.reject_remote_clients(true)
            .max_instances(MAX_INSTANCES)
            .first_pipe_instance(first);
        // SAFETY: `attrs_ptr` zeigt auf eine gueltige SECURITY_ATTRIBUTES, die `sec` am Leben haelt.
        unsafe { opts.create_with_security_attributes_raw(name, sec.attrs_ptr()) }
    }

    // --- Gegenueber pruefen ---------------------------------------------------------------

    fn raw(handle: &impl AsRawHandle) -> *mut c_void {
        handle.as_raw_handle()
    }

    /// Prozess-Kennung des Clients einer Server-Instanz.
    pub fn client_pid(pipe: &impl AsRawHandle) -> io::Result<u32> {
        let mut pid = 0u32;
        // SAFETY: gueltiges Pipe-Handle, Zeiger auf lokale Variable.
        let ok = unsafe { GetNamedPipeClientProcessId(raw(pipe), &mut pid) };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(pid)
        }
    }

    /// Prozess-Kennung des Servers eines Client-Handles.
    pub fn server_pid(pipe: &impl AsRawHandle) -> io::Result<u32> {
        let mut pid = 0u32;
        // SAFETY: wie oben.
        let ok = unsafe { GetNamedPipeServerProcessId(raw(pipe), &mut pid) };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(pid)
        }
    }

    /// Gehoert der Prozess `pid` dem Benutzer `expected`? `Err`: nicht pruefbar (gilt als Nein).
    pub fn pid_belongs_to(pid: u32, expected: &OwnedSid) -> io::Result<bool> {
        Ok(process_user_sid(pid)?.equals(expected))
    }

    // --- Server --------------------------------------------------------------------------------

    /// Bedient die Pipe `name`, bis `server.shutdown()` gerufen wird. `ready` meldet, ob die
    /// erste Instanz angelegt werden konnte (Fehler: Name belegt, keine SID, ...).
    pub async fn serve(
        server: Arc<Server>,
        name: &str,
        ready: Option<oneshot::Sender<io::Result<()>>>,
    ) -> io::Result<()> {
        match current_user_sid() {
            Ok(own) => serve_with(server, name, ready, own).await,
            Err(e) => {
                let copy = io::Error::new(e.kind(), e.to_string());
                if let Some(r) = ready {
                    let _ = r.send(Err(e));
                }
                Err(copy)
            }
        }
    }

    /// Wie `serve`, aber das Gegenueber muss dem Benutzer `peer_sid` gehoeren (die DACL
    /// erlaubt weiterhin nur den aktuellen Benutzer). Fuer Tests der Abweisung.
    pub async fn serve_with(
        server: Arc<Server>,
        name: &str,
        ready: Option<oneshot::Sender<io::Result<()>>>,
        peer_sid: OwnedSid,
    ) -> io::Result<()> {
        let own = peer_sid;
        let setup = (|| -> io::Result<(PipeSecurity, NamedPipeServer)> {
            let mut sec = PipeSecurity::only(current_user_sid()?)?;
            let first = create_instance(name, &mut sec, true)?;
            Ok((sec, first))
        })();
        let (mut sec, first) = match setup {
            Ok(v) => v,
            Err(e) => {
                let copy = io::Error::new(e.kind(), e.to_string());
                if let Some(r) = ready {
                    let _ = r.send(Err(e));
                }
                return Err(copy);
            }
        };
        if let Some(r) = ready {
            let _ = r.send(Ok(()));
        }
        let mut shutdown = server.shutdown_receiver();
        let mut listener: Option<NamedPipeServer> = Some(first);
        loop {
            // Immer eine wartende Instanz bereithalten, damit Clients nie „keine Pipe“ sehen.
            let current = match listener.take() {
                Some(l) => l,
                None => match create_instance(name, &mut sec, false) {
                    Ok(l) => l,
                    Err(e) => {
                        log::warn!("agent_bridge: keine neue Pipe-Instanz: {e}");
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_millis(250)) => continue,
                            _ = shutdown.changed() => break,
                        }
                    }
                },
            };
            let connected = tokio::select! {
                r = current.connect() => r,
                _ = shutdown.changed() => break,
            };
            if let Err(e) = connected {
                log::debug!("agent_bridge: Verbindung beim Annehmen abgebrochen: {e}");
                continue;
            }
            // Naechste Instanz vor der Bearbeitung anlegen.
            match create_instance(name, &mut sec, false) {
                Ok(next) => listener = Some(next),
                Err(e) => log::warn!("agent_bridge: keine neue Pipe-Instanz: {e}"),
            }
            match client_pid(&current).and_then(|pid| pid_belongs_to(pid, &own).map(|same| (pid, same))) {
                Ok((_, true)) => {
                    tokio::spawn(server.clone().handle(current));
                }
                Ok((pid, false)) => {
                    reject(&server, "peer_not_current_user", json!({ "pid": pid }));
                }
                Err(e) => {
                    reject(&server, "peer_unverifiable", json!({ "error": e.to_string() }));
                }
            }
        }
        Ok(())
    }

    fn reject(server: &Arc<Server>, reason: &'static str, detail: serde_json::Value) {
        log::warn!("agent_bridge: Verbindung abgewiesen ({reason})");
        let bridge = server.bridge().clone();
        tokio::task::spawn_blocking(move || bridge.audit_rejected_connection(reason, detail));
    }

    // --- Gegenstelle ------------------------------------------------------------------------------

    /// Oeffnet die Pipe als Client: wartet kurz, wenn alle Instanzen belegt sind; prueft, dass
    /// der Server-Prozess dem eigenen Benutzer gehoert.
    pub fn open_client(name: &str) -> Result<File, ClientError> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let file = loop {
            match OpenOptions::new()
                .read(true)
                .write(true)
                .security_qos_flags(SECURITY_SQOS_PRESENT)
                .open(name)
            {
                Ok(f) => break f,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(40));
                }
                Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => {
                    return Err(ClientError::NotRunning(
                        "Local Voice AI läuft nicht (die Pipe der Agentenbrücke fehlt).".to_string(),
                    ));
                }
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    return Err(ClientError::NotRunning(
                        "Die Agentenbrücke ist ausgelastet (alle Verbindungen belegt).".to_string(),
                    ));
                }
                Err(e) => {
                    return Err(ClientError::NotRunning(format!(
                        "Die Pipe der Agentenbrücke lässt sich nicht öffnen: {e}"
                    )));
                }
            }
        };
        verify_server(&file)?;
        Ok(file)
    }

    /// Prueft, dass der Prozess am anderen Ende dem eigenen Benutzer gehoert.
    pub fn verify_server(file: &File) -> Result<(), ClientError> {
        let own = current_user_sid()
            .map_err(|e| ClientError::UntrustedServer(format!("eigene SID unbekannt: {e}")))?;
        verify_server_for(file, &own)
    }

    /// Wie `verify_server`, gegen eine erwartete SID (Tests).
    pub fn verify_server_for(file: &File, own: &OwnedSid) -> Result<(), ClientError> {
        let untrusted = |why: String| ClientError::UntrustedServer(why);
        let pid = server_pid(file).map_err(|e| untrusted(format!("Server-Prozess unbekannt: {e}")))?;
        match pid_belongs_to(pid, own) {
            Ok(true) => Ok(()),
            Ok(false) => Err(untrusted(
                "Die Pipe gehört einem Prozess eines anderen Benutzers.".to_string(),
            )),
            Err(e) => Err(untrusted(format!("Server-Prozess nicht prüfbar: {e}"))),
        }
    }
}

#[cfg(all(test, windows))]
mod tests;
