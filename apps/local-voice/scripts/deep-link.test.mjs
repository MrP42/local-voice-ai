// TF3: Offline-Vertrag fuer Plattformverdrahtung und Frontend-Handshake.
// Parser und Puffer werden zusaetzlich direkt in deep_link.rs getestet.
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import test from "node:test";

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

test("TF3 ist an allen drei Eingangswegen und am Main-Handshake verdrahtet", () => {
  const lib = read("src-tauri/src/lib.rs");
  assert.match(lib, /deep_link::deep_link_ready/);
  assert.match(lib, /deep_link::State::default\(\)/);
  assert.match(lib, /deep_link::receive_args\(app, args\.iter\(\)\.skip\(1\)\)/);
  assert.match(lib, /deep_link::receive_args\(&app_handle, std::env::args\(\)\.skip\(1\)\)/);
  assert.match(lib, /RunEvent::Opened \{ urls \} =>/);
  assert.match(lib, /deep_link::receive\(app, url\.as_str\(\)\)/);
  assert.match(read("src-tauri/src/main.rs"), /deep_link::is_url_launch/);
  assert.match(read("src/App.tsx"), /connectDeepLinks/);
});

test("Windows: HKCU, portable bleibt frei, Deinstallation prueft Eigentum", () => {
  const nsi = read("src-tauri/nsis/installer.nsi");
  const install = nsi.slice(nsi.indexOf("; --- TF3 REGISTER ---"), nsi.indexOf("; --- END TF3 REGISTER ---"));
  assert.match(install, /\$PortableMode <> 1/);
  assert.match(install, /EnumRegKey \$R8 HKCU/);
  assert.match(install, /EnumRegValue \$R9 HKCU/);
  // Der namenlose Standardwert wird ebenfalls als "" aufgelistet. Nur der
  // Fehlerstatus unterscheidet ihn von einer wirklich leeren Werteliste.
  assert.equal((install.match(/EnumRegValue[^\n]*\n\s*\$\{If\} \$\{Errors\}/g) || []).length, 2);
  assert.match(install, /WriteRegStr HKCU "Software\\Classes\\localvoice" "URL Protocol" ""/);
  assert.match(install, /WriteRegStr HKCU .*shell\\open\\command/);
  const uninstall = nsi.slice(nsi.indexOf("; --- TF3 UNREGISTER ---"), nsi.indexOf("; --- END TF3 UNREGISTER ---"));
  assert.match(uninstall, /ReadRegStr \$R7 HKCU .*shell\\open\\command/);
  assert.match(uninstall, /\$\{If\} \$R7 ==/);
  assert.doesNotMatch(uninstall, /DeleteRegKey HKCU/);
  assert.match(uninstall, /!insertmacro TF3_DELETE_EMPTY_KEY/);
  assert.match(uninstall, /ClearErrors\s+ReadRegStr \$R7 HKCU "Software\\Classes\\localvoice" "URL Protocol"\s+\$\{IfNot\} \$\{Errors\}/);
});

test("Neuladen sperrt die Zustellung bis zum neuen Frontend-Handshake", () => {
  assert.match(read("src-tauri/src/lib.rs"), /\.on_page_load\(deep_link::page_started\)/);
  assert.match(read("src-tauri/src/deep_link.rs"), /PageLoadEvent::Started/);
  assert.match(read("src-tauri/src/deep_link.rs"), /inbox.ready = false/);
});

test("macOS: Info.plist wird explizit zusammengefuehrt, keine neue Abhaengigkeit", () => {
  assert.equal(JSON.parse(read("src-tauri/tauri.conf.json")).bundle.macOS.infoPlist, "Info.plist");
  assert.match(read("src-tauri/Info.plist"), /<key>CFBundleURLSchemes<\/key>\s*<array>\s*<string>localvoice<\/string>/);
  assert.doesNotMatch(read("src-tauri/Cargo.toml"), /tauri-plugin-deep-link/);
});

const receiverPath = new URL("../src/lib/deepLink.ts", import.meta.url);
test("Frontend hoert vor ready, merkt nur letztes Ziel, fuehrt keine Aktion aus", async () => {
  assert.ok(existsSync(receiverPath), "Frontend-Empfaenger fehlt");
  const { connectDeepLinks, deepLinkTarget } = await import(receiverPath.href);
  let handler;
  const order = [];
  let unlistened = 0;
  let notifications = 0;
  const unsubscribe = deepLinkTarget.subscribe(() => { notifications++; });
  const stop = connectDeepLinks(
    async (name, fn) => {
      assert.equal(name, "deep-link-open");
      order.push("listen");
      handler = fn;
      return () => { unlistened++; };
    },
    async () => {
      order.push("ready");
      handler({ payload: { ziel: "terminfindung", id: "T-TEST" } });
    },
  );
  await new Promise(setImmediate);
  assert.deepEqual(order, ["listen", "ready"]);
  assert.deepEqual(deepLinkTarget.getSnapshot(), { ziel: "terminfindung", id: "T-TEST" });
  handler({ payload: { ziel: "terminfindung", id: "T-AB12" } });
  handler({ payload: { ziel: "fremd", id: "T-TEST" } });
  for (const id of ["T-TEST\n", "T-test", "T-TEST?x", "T-%54EST", "T-ABCDE", "T-ABC"]) {
    handler({ payload: { ziel: "terminfindung", id } });
  }
  assert.deepEqual(deepLinkTarget.getSnapshot(), { ziel: "terminfindung", id: "T-AB12" });
  assert.equal(notifications, 2);
  stop();
  assert.equal(unlistened, 1);
  handler({ payload: { ziel: "terminfindung", id: "T-STOP" } });
  assert.equal(notifications, 2);
  unsubscribe();
});

test("Deep-Link-Logs enthalten ausschliesslich Zaehler", () => {
  const rust = read("src-tauri/src/deep_link.rs");
  const logs = [...rust.matchAll(/log::\w+!\(([\s\S]*?)\);/g)];
  assert.ok(logs.length > 0);
  for (const [, args] of logs) {
    assert.match(args, /^"deep-link ignored count=\{\}", inbox\.ignored$/);
  }
  assert.doesNotMatch(read("src/lib/deepLink.ts"), /console\./);
});

test("IPC-Fehler werden ohne Nutzlast-Logs abgefangen und Listener entfernt", async () => {
  const { connectDeepLinks } = await import(receiverPath.href);
  let stopped = 0;
  const stop = connectDeepLinks(
    async () => () => { stopped++; },
    async () => { throw new Error("synthetic IPC failure"); },
  );
  await new Promise(setImmediate);
  assert.equal(stopped, 1);
  stop();
  assert.equal(stopped, 1);
});

test("Unmount waehrend listen: keine Bereitschaft und kein Listener-Leck", async () => {
  assert.ok(existsSync(receiverPath), "Frontend-Empfaenger fehlt");
  const { connectDeepLinks } = await import(receiverPath.href);
  let resolve;
  let unlistened = 0;
  let ready = 0;
  const stop = connectDeepLinks(
    () => new Promise((done) => { resolve = done; }),
    async () => { ready++; },
  );
  stop();
  resolve(() => { unlistened++; });
  await new Promise(setImmediate);
  assert.equal(ready, 0);
  assert.equal(unlistened, 1);
});
