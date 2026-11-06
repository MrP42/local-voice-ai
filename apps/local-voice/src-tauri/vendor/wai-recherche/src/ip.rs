//! Adresspruefung gegen SSRF. Der Pruefer ist injizierbar: Tests duerfen Loopback gezielt erlauben,
//! alle uebrigen Sperren bleiben dann bestehen.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::fehler::RechercheFehler;

#[derive(Debug, Clone, Copy, Default)]
pub struct IpPruefer {
    loopback_erlaubt: bool,
}

impl IpPruefer {
    /// Standard fuer den Betrieb: nur oeffentliche Adressen.
    pub fn streng() -> Self {
        Self::default()
    }

    /// NUR fuer Tests und lokale Entwicklung: Loopback (127/8, ::1) wird zugelassen.
    pub fn loopback_erlaubt() -> Self {
        Self {
            loopback_erlaubt: true,
        }
    }

    pub fn pruefe(&self, ip: IpAddr) -> Result<(), RechercheFehler> {
        let ip = normalisiere(ip);
        if self.loopback_erlaubt && ip.is_loopback() {
            return Ok(());
        }
        if ist_gesperrt(ip) {
            return Err(RechercheFehler::AdresseNichtErlaubt);
        }
        Ok(())
    }
}

/// IPv4-mapped (`::ffff:a.b.c.d`), IPv4-kompatibel, 6to4 und NAT64 (`64:ff9b::/96`) auf die eingebettete IPv4 zurueckfuehren.
fn normalisiere(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return IpAddr::V4(v4);
            }
            let s = v6.segments();
            let o = v6.octets();
            // IPv4-kompatibel ::a.b.c.d (ohne :: und ::1)
            if s[..6] == [0; 6] && !v6.is_unspecified() && !v6.is_loopback() {
                return IpAddr::V4(Ipv4Addr::new(o[12], o[13], o[14], o[15]));
            }
            // 6to4 2002::/16: eingebettete IPv4 in Bytes 2..6
            if s[0] == 0x2002 {
                return IpAddr::V4(Ipv4Addr::new(o[2], o[3], o[4], o[5]));
            }
            if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
                return IpAddr::V4(Ipv4Addr::new(o[12], o[13], o[14], o[15]));
            }
            IpAddr::V6(v6)
        }
        v4 => v4,
    }
}

fn ist_gesperrt(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => gesperrt_v4(v4),
        IpAddr::V6(v6) => gesperrt_v6(v6),
    }
}

fn gesperrt_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || o[0] == 0
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // CGNAT 100.64/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24
        || (o[0] == 192 && o[1] == 0 && o[2] == 2)
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // Benchmark 198.18/15
        || (o[0] == 192 && o[1] == 88 && o[2] == 99) // 6to4-Relay 192.88.99.0/24
        || o[0] >= 240 // reserviert
}

fn gesperrt_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (s[0] & 0xfe00) == 0xfc00 // ULA fc00::/7
        || (s[0] & 0xffc0) == 0xfe80 // Link-Local fe80::/10
        || (s[0] & 0xffc0) == 0xfec0 // Site-Local (veraltet)
        || (s[0] == 0x2001 && s[1] == 0x0db8) // Dokumentation
        || (s[0] == 0x2001 && (s[1] & 0xfe00) == 0) // 2001::/23 inkl. Teredo 2001:0000::/32
        || (s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 1) // 64:ff9b:1::/48
        || (s[0] == 0x0100 && s[1..4] == [0, 0, 0]) // Discard 100::/64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gesperrt(s: &str) -> bool {
        IpPruefer::streng().pruefe(s.parse().unwrap()).is_err()
    }

    #[test]
    fn gesperrte_adressen() {
        for a in [
            "127.0.0.1",
            "127.255.0.9",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.255",
            "0.0.0.0",
            "0.1.2.3",
            "224.0.0.1",
            "255.255.255.255",
            "240.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::7f00:1",
            "::7f00:1",
            "::a00:1",
            "::a9fe:a9fe",
            "2002:7f00:1::",
            "2002:a00:1::1",
            "2002:a9fe:a9fe::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "2001:1::1",
            "2001:1ff::1",
            "64:ff9b:1::1",
            "192.88.99.1",
        ] {
            assert!(gesperrt(a), "{a} muss gesperrt sein");
        }
    }

    #[test]
    fn oeffentliche_adressen() {
        for a in [
            "8.8.8.8",
            "93.184.216.34",
            "172.32.0.1",
            "100.128.0.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
            "2002:808:808::1",
            "::808:808",
            "2001:200::1",
        ] {
            assert!(!gesperrt(a), "{a} muss erlaubt sein");
        }
    }

    #[test]
    fn loopback_freigabe_laesst_rest_gesperrt() {
        let p = IpPruefer::loopback_erlaubt();
        assert!(p.pruefe("127.0.0.1".parse().unwrap()).is_ok());
        assert!(p.pruefe("::1".parse().unwrap()).is_ok());
        assert!(p.pruefe("::ffff:127.0.0.1".parse().unwrap()).is_ok());
        assert!(p.pruefe("10.0.0.1".parse().unwrap()).is_err());
        assert!(p.pruefe("169.254.169.254".parse().unwrap()).is_err());
        assert!(p.pruefe("0.0.0.0".parse().unwrap()).is_err());
    }
}
