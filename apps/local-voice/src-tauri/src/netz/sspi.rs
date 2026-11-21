//! Windows-Anmeldung am Weiterleitungsproxy mit Standardanmeldeinformationen (lva#641).

use std::sync::Arc;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    SEC_E_OK, SEC_I_COMPLETE_AND_CONTINUE, SEC_I_COMPLETE_NEEDED, SEC_I_CONTINUE_NEEDED,
};
use windows::Win32::Security::Authentication::Identity::{
    AcquireCredentialsHandleW, CompleteAuthToken, DeleteSecurityContext, FreeContextBuffer,
    FreeCredentialsHandle, InitializeSecurityContextW, SecBuffer, SecBufferDesc,
    ISC_REQ_ALLOCATE_MEMORY, ISC_REQ_CONNECTION, ISC_REQ_FLAGS, SECBUFFER_TOKEN, SECBUFFER_VERSION,
    SECPKG_CRED_OUTBOUND, SECURITY_NATIVE_DREP,
};
use windows::Win32::Security::Credentials::SecHandle;

use super::proxy_auth::{AuthFabrik, AuthFehler, ProxyAuthentifizierer};

pub struct SspiAuthentifizierer {
    paket: Option<&'static str>,
    anmeldedaten: SecHandle,
    hat_anmeldedaten: bool,
    kontext: SecHandle,
    hat_kontext: bool,
    gescheitert: bool,
}

impl SspiAuthentifizierer {
    pub fn neu() -> Self {
        Self {
            paket: None,
            anmeldedaten: SecHandle::default(),
            hat_anmeldedaten: false,
            kontext: SecHandle::default(),
            hat_kontext: false,
            gescheitert: false,
        }
    }
}

impl Default for SspiAuthentifizierer {
    fn default() -> Self {
        Self::neu()
    }
}

// SSPI besitzt diesen Puffer auch dann, wenn ein späterer Schritt fehlschlägt.
struct Ausgabepuffer(SecBuffer);

impl Drop for Ausgabepuffer {
    fn drop(&mut self) {
        if !self.0.pvBuffer.is_null() {
            // Der Zeiger stammt von SSPI und bleibt bis zu dieser einmaligen Freigabe gültig.
            let _ = unsafe { FreeContextBuffer(self.0.pvBuffer) };
        }
    }
}

impl ProxyAuthentifizierer for SspiAuthentifizierer {
    fn schritt(
        &mut self,
        proxy_host: &str,
        schema: &str,
        server_token: Option<&[u8]>,
    ) -> Result<Vec<u8>, AuthFehler> {
        let paket = if schema.eq_ignore_ascii_case("Negotiate") {
            "Negotiate"
        } else if schema.eq_ignore_ascii_case("NTLM") {
            "NTLM"
        } else {
            return Err(AuthFehler::NichtVerfuegbar);
        };
        if self.paket.is_some_and(|alt| alt != paket) {
            return Err(AuthFehler::NichtVerfuegbar);
        }
        if self.gescheitert {
            return Err(AuthFehler::Abgelehnt);
        }
        // Nach einem Fehler darf dieselbe Instanz keinen neuen Handshake beginnen.
        self.gescheitert = true;
        self.paket = Some(paket);
        if !self.hat_anmeldedaten {
            let paket: Vec<u16> = paket.encode_utf16().chain(Some(0)).collect();
            // Paket und Handle leben während des Aufrufs; ohne Auth-Daten nutzt SSPI den Benutzer.
            unsafe {
                AcquireCredentialsHandleW(
                    PCWSTR::null(),
                    PCWSTR(paket.as_ptr()),
                    SECPKG_CRED_OUTBOUND,
                    None,
                    None,
                    None,
                    None,
                    &mut self.anmeldedaten,
                    None,
                )
            }
            .map_err(|_| AuthFehler::Abgelehnt)?;
            self.hat_anmeldedaten = true;
        }

        // Eingebettete Nullzeichen würden den Zielnamen für SSPI abschneiden.
        if proxy_host.contains('\0') {
            return Err(AuthFehler::Abgelehnt);
        }
        let ziel: Vec<u16> = format!("HTTP/{proxy_host}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut eingabe = server_token.map(<[u8]>::to_vec).unwrap_or_default();
        let mut eingabepuffer = SecBuffer {
            cbBuffer: u32::try_from(eingabe.len()).map_err(|_| AuthFehler::Abgelehnt)?,
            BufferType: SECBUFFER_TOKEN,
            pvBuffer: eingabe.as_mut_ptr().cast(),
        };
        let eingabebeschreibung = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: 1,
            pBuffers: &mut eingabepuffer,
        };
        let mut ausgabe = Ausgabepuffer(SecBuffer {
            BufferType: SECBUFFER_TOKEN,
            ..SecBuffer::default()
        });
        let mut ausgabebeschreibung = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: 1,
            pBuffers: &mut ausgabe.0,
        };
        let mut attribute = 0;
        let kontext = &mut self.kontext as *mut SecHandle;
        // Alle Puffer, Beschreibungen und Handles leben bis nach dem Aufruf. Ein bestehender
        // Kontext wird über denselben Handle fortgesetzt; SSPI darf den Ausgabezeiger setzen.
        let status = unsafe {
            InitializeSecurityContextW(
                Some(&self.anmeldedaten),
                self.hat_kontext.then_some(kontext.cast_const()),
                Some(ziel.as_ptr()),
                ISC_REQ_FLAGS(ISC_REQ_CONNECTION.0 | ISC_REQ_ALLOCATE_MEMORY.0),
                0,
                SECURITY_NATIVE_DREP,
                server_token.map(|_| &eingabebeschreibung as *const SecBufferDesc),
                0,
                Some(kontext),
                Some(&mut ausgabebeschreibung),
                &mut attribute,
                None,
            )
        };
        let erfolg = status == SEC_E_OK
            || status == SEC_I_CONTINUE_NEEDED
            || status == SEC_I_COMPLETE_NEEDED
            || status == SEC_I_COMPLETE_AND_CONTINUE;
        // Auch einen vor einem Fehler bereits befüllten Handle beim Drop freigeben.
        self.hat_kontext |= erfolg || self.kontext.dwLower != 0 || self.kontext.dwUpper != 0;
        if !erfolg {
            return Err(AuthFehler::Abgelehnt);
        }
        if status == SEC_I_COMPLETE_NEEDED || status == SEC_I_COMPLETE_AND_CONTINUE {
            // Kontext und SSPI-Puffer sind noch gültig und werden erst beim Drop freigegeben.
            unsafe { CompleteAuthToken(&self.kontext, &ausgabebeschreibung) }
                .map_err(|_| AuthFehler::Abgelehnt)?;
        }
        let token = if ausgabe.0.cbBuffer == 0 {
            Vec::new()
        } else {
            let laenge = ausgabe.0.cbBuffer as usize;
            if ausgabe.0.pvBuffer.is_null() || laenge > isize::MAX as usize {
                return Err(AuthFehler::Abgelehnt);
            }
            // SSPI garantiert cbBuffer lesbare Bytes; die Kopie lebt über die Freigabe hinaus.
            unsafe { std::slice::from_raw_parts(ausgabe.0.pvBuffer.cast::<u8>(), laenge) }.to_vec()
        };
        self.gescheitert = false;
        Ok(token)
    }
}

impl Drop for SspiAuthentifizierer {
    fn drop(&mut self) {
        if self.hat_kontext {
            // Der eigene Handle bleibt bis zur einmaligen Freigabe in diesem Drop gültig.
            let _ = unsafe { DeleteSecurityContext(&self.kontext) };
        }
        if self.hat_anmeldedaten {
            // Die Anmeldedaten gehören dieser Instanz und überleben die Kontextfreigabe.
            let _ = unsafe { FreeCredentialsHandle(&self.anmeldedaten) };
        }
    }
}

pub fn fabrik() -> AuthFabrik {
    Arc::new(|| Box::new(SspiAuthentifizierer::neu()))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn neu_und_drop_ohne_schritt() {
        let auth = SspiAuthentifizierer::neu();
        assert!(auth.paket.is_none());
        assert!(!auth.hat_anmeldedaten);
        assert!(!auth.hat_kontext);
        drop(auth);
    }

    #[test]
    fn unbekanntes_schema_ohne_sspi() {
        let mut auth = SspiAuthentifizierer::neu();
        assert_eq!(
            auth.schritt("proxy.example.invalid", "Basic", None),
            Err(AuthFehler::NichtVerfuegbar)
        );
        assert!(auth.paket.is_none());
        assert!(!auth.hat_anmeldedaten);
        assert!(!auth.hat_kontext);
        assert!(!auth.gescheitert);
    }

    #[test]
    fn ntlm_type_eins_ohne_netz() {
        let mut auth = SspiAuthentifizierer::neu();
        if let Ok(token) = auth.schritt("proxy.example.invalid", "NTLM", None) {
            assert!(token.starts_with(b"NTLMSSP\0"));
            assert_eq!(token.get(8..12), Some([1, 0, 0, 0].as_slice()));
        }
        // Lokale Richtlinien können NTLM sperren; AuthFehler ist deshalb ebenfalls zulässig.
    }

    #[test]
    fn muell_challenge_wird_abgelehnt() {
        let mut auth = SspiAuthentifizierer::neu();
        let _ = auth.schritt("proxy.example.invalid", "NTLM", None);
        assert_eq!(
            auth.schritt("proxy.example.invalid", "NTLM", Some(b"muell")),
            Err(AuthFehler::Abgelehnt)
        );
    }

    #[test]
    fn schemawechsel_wird_abgelehnt() {
        let mut auth = SspiAuthentifizierer::neu();
        let _ = auth.schritt("proxy.example.invalid", "NTLM", None);
        assert_eq!(
            auth.schritt("proxy.example.invalid", "Negotiate", None),
            Err(AuthFehler::NichtVerfuegbar)
        );
    }

    #[test]
    fn parallele_instanzen_und_wiederholter_drop() {
        let fabrik = fabrik();
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let fabrik = fabrik.clone();
                std::thread::spawn(move || {
                    for _ in 0..4 {
                        drop(SspiAuthentifizierer::neu());
                        let mut auth = fabrik();
                        let _ = auth.schritt("proxy.example.invalid", "NTLM", None);
                        drop(auth);
                    }
                })
            })
            .collect();
        for thread in threads {
            assert!(thread.join().is_ok());
        }
    }
}
