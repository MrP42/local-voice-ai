//! Ende-zu-Ende gegen einen laufenden Hub (HUB_E2E=http://127.0.0.1:8099). Das iPhone
//! wird hier mit den Protokollfunktionen nachgebildet.
use crate::sync::{account, client, pairing::*, pairing_proto as p};
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use serde_json::{json, Value};
use x25519_dalek::StaticSecret;

fn hub() -> Option<String> {
    std::env::var("HUB_E2E").ok()
}

async fn login(url: &str, email: &str, key: [u8; 32]) {
    let l = client::login(url, email, "korrekt-123", "Arbeitsplatz-PC")
        .await
        .unwrap();
    *account::CFG.lock().unwrap() = Some(account::Cfg {
        url: url.into(),
        token: l.token,
        user_id: l.user.id,
        user_email: l.user.email,
        key,
    });
}

fn param(uri: &str, k: &str) -> String {
    let q = uri.split_once('?').unwrap().1;
    url::form_urlencoded::parse(q.as_bytes())
        .find(|(a, _)| a == k)
        .unwrap()
        .1
        .into_owned()
}

struct Phone {
    http: reqwest::Client,
    hub: String,
    id: String,
    q: [u8; 32],
    d_pk: [u8; 32],
    p_sk: StaticSecret,
    p_pk: [u8; 32],
    n_p: [u8; 32],
    poll: String,
}

impl Phone {
    async fn scan(uri: &str, name: &str, wrong_q: bool) -> Phone {
        assert_eq!(param(uri, "v"), "2");
        let hub = param(uri, "h");
        let id = param(uri, "p");
        let secret = param(uri, "s");
        let mut q = p::b64u32(&param(uri, "q")).unwrap();
        let d_pk = p::b64u32(&param(uri, "k")).unwrap();
        if wrong_q {
            q[0] ^= 1;
        }
        let p_sk = StaticSecret::from([9u8; 32]);
        let p_pk = p::public_key(&p_sk);
        let n_p = [5u8; 32];
        let c = p::commitment(&id, &p_pk, &n_p, name);
        let mac = p::claim_mac(&q, &id, &c);
        let http = reqwest::Client::new();
        let r: Value = http.post(format!("{hub}/api/v1/voice/pair/claim"))
            .json(&json!({"pairing_id": id, "secret": secret, "phone_commit": p::b64u(&c), "phone_mac": p::b64u(&mac), "device_name": name}))
            .send().await.unwrap().json().await.unwrap();
        let poll = r["poll_token"].as_str().unwrap().to_string();
        Phone {
            http,
            hub,
            id,
            q,
            d_pk,
            p_sk,
            p_pk,
            n_p,
            poll,
        }
    }
    async fn result(&self) -> (u16, Value) {
        let r = self
            .http
            .get(format!("{}/api/v1/voice/pair/{}/result", self.hub, self.id))
            .header("X-Pairing-Token", &self.poll)
            .send()
            .await
            .unwrap();
        let st = r.status().as_u16();
        (st, r.json().await.unwrap_or(Value::Null))
    }
    async fn reveal(&self) -> u16 {
        self.http
            .post(format!("{}/api/v1/voice/pair/{}/reveal", self.hub, self.id))
            .header("X-Pairing-Token", &self.poll)
            .json(&json!({"phone_pub": p::b64u(&self.p_pk), "phone_nonce": p::b64u(&self.n_p)}))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }
    fn s(&self) -> zeroize::Zeroizing<[u8; 32]> {
        let z = p::shared(&self.p_sk, &self.d_pk).unwrap();
        p::session_key(&z, &self.q, &self.d_pk, &self.p_pk, &self.id)
    }
    async fn confirm(&self, tag: [u8; 32]) -> u16 {
        self.http
            .post(format!(
                "{}/api/v1/voice/pair/{}/confirm",
                self.hub, self.id
            ))
            .header("X-Pairing-Token", &self.poll)
            .json(&json!({"confirm": p::b64u(&tag)}))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }
}

fn st<'a>(s: &'a PairingState) -> tauri::State<'a, PairingState> {
    tauri::State(s)
}

#[tokio::test(flavor = "current_thread")]
async fn e2e_alle_pfade() {
    let Some(url) = hub() else { return };
    let k = [0x42u8; 32];
    login(&url, "patrick@example.org", k).await;

    // 1) Erfolgsfall
    let ps = PairingState::default();
    let v = pair_start(tauri::AppHandle, st(&ps)).await.unwrap();
    assert_eq!(v.stage, PairStage::Qr);
    assert!(v.qr_svg.unwrap().contains("<svg"));
    assert!(v.remaining_s > 100);
    let uri = ps.scanned_uri().await.unwrap();
    let phone = Phone::scan(&uri, "iPhone von Patrick", false).await;
    let v = pair_poll(st(&ps)).await.unwrap();
    assert_eq!(
        (v.stage, v.device_name.as_deref()),
        (PairStage::Qr, Some("iPhone von Patrick")),
        "{:?}",
        v.reason
    );
    let (code, r) = phone.result().await;
    assert_eq!(
        (code, r["status"].as_str()),
        (200, Some("challenge")),
        "{r}"
    );
    let n_d = p::b64u32(r["desktop_nonce"].as_str().unwrap()).unwrap();
    assert_eq!(phone.reveal().await, 200);
    let v = pair_poll(st(&ps)).await.unwrap();
    assert_eq!(v.stage, PairStage::Sas, "{:?}", v.reason);
    let phone_sas = p::sas(&phone.id, &phone.d_pk, &phone.p_pk, &phone.n_p, &n_d);
    assert_eq!(v.sas.as_deref(), Some(phone_sas.as_str()));
    let v = pair_approve(st(&ps)).await.unwrap();
    assert_eq!(v.stage, PairStage::WaitConfirm, "{:?}", v.reason);
    let (code, r) = phone.result().await;
    assert_eq!(code, 200, "{r}");
    let token = r["token"].as_str().unwrap().to_string();
    let raw = base64::engine::general_purpose::STANDARD
        .decode(r["sealed"].as_str().unwrap())
        .unwrap();
    let s = phone.s();
    let plain = chacha20poly1305::XChaCha20Poly1305::new((&*s).into())
        .decrypt(
            chacha20poly1305::XNonce::from_slice(&raw[..24]),
            Payload {
                msg: &raw[24..],
                aad: &p::seal_aad(&phone.id, &phone.d_pk, &phone.p_pk),
            },
        )
        .expect("iPhone oeffnet sealed");
    let pl: Value = serde_json::from_slice(&plain).unwrap();
    assert_eq!(pl["k"], p::b64u(&k));
    assert_eq!(pl["user_id"], r["user"]["id"]);
    assert_eq!(pl["email"], "patrick@example.org");
    let tag = p::b64u32(&{
        use hmac::Mac;
        let mut m = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(&*s).unwrap();
        m.update(b"LV-PAIR-CONFIRM-v2");
        m.update(phone.id.as_bytes());
        p::b64u(&m.finalize().into_bytes())
    })
    .unwrap();
    assert_eq!(phone.confirm(tag).await, 200);
    let v = pair_poll(st(&ps)).await.unwrap();
    assert_eq!(v.stage, PairStage::Connected, "{:?}", v.reason);
    // Das neue Token synchronisiert.
    let status = client::HubClient::new(&url, &token).pull(0, 10).await;
    assert!(
        status.is_ok(),
        "neues Geraetetoken nutzbar: {:?}",
        status.err()
    );
    let devs = sync_devices(tauri::AppHandle).await.unwrap();
    let me = devs.iter().find(|d| d.current).unwrap().id;
    let phone_dev = devs
        .iter()
        .filter(|d| d.name == "iPhone von Patrick")
        .max_by_key(|d| d.id)
        .unwrap();
    assert_eq!(phone_dev.paired_by, Some(me));
    assert!(phone_dev.via_pairing);

    // 2) Falsches q (Hub oder Dritter ohne QR-Geheimnis): Desktop bricht ab
    let ps = PairingState::default();
    pair_start(tauri::AppHandle, st(&ps)).await.unwrap();
    let uri = ps.scanned_uri().await.unwrap();
    let _evil = Phone::scan(&uri, "Fremd", true).await;
    let v = pair_poll(st(&ps)).await.unwrap();
    assert_eq!(v.stage, PairStage::Aborted);
    assert!(v.reason.unwrap().contains("anderes Gerät"));

    // 3) Falsche Bestaetigung: "nicht bestaetigt" mit Angebot zum Abmelden, Widerruf wirkt
    let ps = PairingState::default();
    pair_start(tauri::AppHandle, st(&ps)).await.unwrap();
    let uri = ps.scanned_uri().await.unwrap();
    let phone = Phone::scan(&uri, "iPad Kueche", false).await;
    pair_poll(st(&ps)).await.unwrap();
    phone.result().await;
    assert_eq!(phone.reveal().await, 200);
    assert_eq!(pair_poll(st(&ps)).await.unwrap().stage, PairStage::Sas);
    pair_approve(st(&ps)).await.unwrap();
    let (_, r) = phone.result().await;
    let token = r["token"].as_str().unwrap().to_string();
    assert_eq!(phone.confirm([1u8; 32]).await, 200);
    let v = pair_poll(st(&ps)).await.unwrap();
    assert_eq!(v.stage, PairStage::Unconfirmed);
    let sus = v.suspect.expect("verdaechtiges Geraet");
    assert_eq!(sus.name, "iPad Kueche");
    sync_revoke_device(tauri::AppHandle, sus.id).await.unwrap();
    assert!(matches!(
        client::HubClient::new(&url, &token).pull(0, 10).await,
        Err(client::HubError::Unauthorized)
    ));

    // 4) Nein beim Vergleichscode: iPhone erhaelt 403
    let ps = PairingState::default();
    pair_start(tauri::AppHandle, st(&ps)).await.unwrap();
    let uri = ps.scanned_uri().await.unwrap();
    let phone = Phone::scan(&uri, "iPhone Test", false).await;
    pair_poll(st(&ps)).await.unwrap();
    phone.result().await;
    phone.reveal().await;
    assert_eq!(pair_poll(st(&ps)).await.unwrap().stage, PairStage::Sas);
    assert_eq!(
        pair_cancel(st(&ps)).await.unwrap().stage,
        PairStage::Aborted
    );
    assert_eq!(phone.result().await.0, 403);

    // 5) Nach Abbruch keine Kopplung mehr offen
    assert_eq!(pair_poll(st(&ps)).await.unwrap().stage, PairStage::Aborted);

    // 6) Schliessen nach der Freigabe, bevor das iPhone abholt: Freigabe zurueckgezogen
    let ps = PairingState::default();
    pair_start(tauri::AppHandle, st(&ps)).await.unwrap();
    let uri = ps.scanned_uri().await.unwrap();
    let phone = Phone::scan(&uri, "iPhone Spaet", false).await;
    pair_poll(st(&ps)).await.unwrap();
    phone.result().await;
    phone.reveal().await;
    assert_eq!(pair_poll(st(&ps)).await.unwrap().stage, PairStage::Sas);
    assert_eq!(
        pair_approve(st(&ps)).await.unwrap().stage,
        PairStage::WaitConfirm
    );
    let v = pair_cancel(st(&ps)).await.unwrap();
    assert_eq!(v.stage, PairStage::Aborted);
    assert!(v.reason.unwrap().contains("zurückgezogen"));
    assert_eq!(phone.result().await.0, 403);

    // 7) Schliessen nach Abholung und gueltiger Bestaetigung: "Verbunden", kein Abmeldeangebot
    let ps = PairingState::default();
    pair_start(tauri::AppHandle, st(&ps)).await.unwrap();
    let uri = ps.scanned_uri().await.unwrap();
    let phone = Phone::scan(&uri, "iPhone Gut", false).await;
    pair_poll(st(&ps)).await.unwrap();
    phone.result().await;
    phone.reveal().await;
    pair_poll(st(&ps)).await.unwrap();
    pair_approve(st(&ps)).await.unwrap();
    assert_eq!(phone.result().await.0, 200);
    let s = phone.s();
    let tag = {
        use hmac::Mac;
        let mut m = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(&*s).unwrap();
        m.update(b"LV-PAIR-CONFIRM-v2");
        m.update(phone.id.as_bytes());
        let mut t = [0u8; 32];
        t.copy_from_slice(&m.finalize().into_bytes());
        t
    };
    assert_eq!(phone.confirm(tag).await, 200);
    let v = pair_cancel(st(&ps)).await.unwrap();
    assert_eq!(v.stage, PairStage::Connected, "{:?}", v.reason);
}
