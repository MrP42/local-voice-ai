// Lva#509/#511: Auszüge und Äußerungen stehen nie im Log. Geprüft werden ALLE Log-Aufrufe in den Dateien
// des Inhaltszugriffs (Rust: `app_context/`, `voice_call/`, `commands/voice_tools.rs`; Oberfläche:
// `components/voice-call/`). Ein kleiner Scanner zerlegt den Quelltext in Bezeichner, Zeichenketten
// (Rust: auch Raw-Strings und Zeichenliterale; TypeScript: auch Template-Literale samt ihrer `${…}`-
// Ausdrücke, die als Code gelesen werden), Kommentare und Klammern. Ein Log-Aufruf ist eine Namensfolge
// (`log :: info !`, `console . log`) gefolgt von einer öffnenden Klammer, über beliebigen Leerraum und
// Zeilenumbrüche. Er besteht nur, wenn seine Argumente reine Textliterale ohne Platzhalter sind; alles
// andere fällt auf, auch ohne Inhaltsnamen. Ausnahmen stehen in `ERLAUBT`, gebunden an Datei und den
// vollständigen Aufruf samt Pfad, höchstens einmal je Eintrag.
// Rust-Testcode wird nur bei exakt `#[cfg(test)]`, `#[test]` und `…::test` per Klammertiefe blockweise
// übersprungen (ein inneres `#![cfg(test)]` höchstens den umschließenden Block); jedes andere Attribut
// blendet nichts aus. Code danach zählt wieder.
//
// Grenzen, ehrlich: (1) Die Prüfung ist ein Stolperdraht gegen versehentliches Loggen von Inhalten in
// den genannten Verzeichnissen und kein Beweis gegen absichtliche Umgehung. (2) Unbekannte oder nicht
// sicher einordbare Konstrukte sind Fehler, kein Durchlass (offene Literale, unbestimmbare Blockgrenzen,
// Umbenennung von Log-Makros per `use … as`, `macro_rules!` mit Log-Makro, `console` ohne Aufruf).
// (3) Die strukturelle Durchsetzung über Lint-Regeln mit echten Parsern ist als eigene Aufgabe
// vermerkt: lva#518.
// Lauf: node --test scripts/log-hygiene.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const wurzel = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Dateien eines Ordners (nicht rekursiv) ohne reine Testdateien (`tests.rs`, `*.test.*`). */
function ordner(rel, endung) {
  return readdirSync(join(wurzel, rel))
    .filter((f) => endung.test(f) && !/^tests\.rs$|\.test\./.test(f))
    .map((f) => `${rel}/${f}`);
}
const RUST = [
  ...ordner("src-tauri/src/app_context", /\.rs$/),
  ...ordner("src-tauri/src/voice_call", /\.rs$/),
  "src-tauri/src/commands/voice_tools.rs",
];
const TS = ordner("src/components/voice-call", /\.tsx?$/);

/**
 * Geprüfte Ausnahmen. Schlüssel: `Datei|Aufruf`, der Aufruf als Folge seiner Token mit je einem
 * Leerzeichen dazwischen (so, wie `aufrufText` ihn bildet). Jeder Eintrag darf in seiner Datei genau
 * einmal vorkommen; ein zweiter Aufruf, derselbe Text in einer anderen Datei oder ein Eintrag ohne
 * Fundstelle lässt die Prüfung scheitern.
 */
const ERLAUBT = new Map([
  [
    'src-tauri/src/commands/voice_tools.rs|log :: warn ! ( "voice_tools: {what} nicht lesbar: {e}" )',
    "`what` ist ein fester Bereichsname, `e` ein Dateifehler des Systems",
  ],
  [
    'src/components/voice-call/VoiceCallPage.tsx|console . warn ( "story send failed" , e )',
    "nur das Fehlerobjekt des Sendens, kein Gesprächstext",
  ],
]);

/** Namen, die Inhalt tragen (nur für den Plausibilitätstest der Erlaubnisliste). */
const INHALT =
  /\b(excerpt|excerpts|question|transcript|utterance|text|content|answer|hits?|prompt|message|messages|req|sentence|piece|userText|heard)\b/i;

const RUST_LOGS = new Set([
  "log",
  "trace",
  "debug",
  "info",
  "warn",
  "error",
  "print",
  "println",
  "eprint",
  "eprintln",
  "dbg",
]);
const TS_LOGS = new Set(["log", "info", "warn", "error", "debug", "trace"]);

// ---------------------------------------------------------------------------------------------
// Scanner
// ---------------------------------------------------------------------------------------------

class Unsicher extends Error {}

const IDSTART = /[\p{L}_$]/u;
const IDREST = /[\p{L}\p{N}_$]/u;

/**
 * Zerlegt Quelltext in Token `{ t: "id" | "str" | "p", v, interp? }`. `str` trägt den Rohtext
 * (mit Anführungszeichen), `interp` ist bei Template-Literalen mit `${` wahr. Kommentare und
 * Leerraum entfallen. Ein nicht abschließbares Literal oder Kommentar wirft `Unsicher`.
 */
function tokenize(src, sprache) {
  const rust = sprache === "rust";
  const out = [];
  let i = 0;
  const n = src.length;
  const fehler = (was) => {
    const zeile = src.slice(0, i).split("\n").length;
    throw new Unsicher(`${was} (Zeile ${zeile})`);
  };
  /** Letztes Token erlaubt danach einen Regex-Ausdruck (TypeScript)? */
  const regexMoeglich = () => {
    const l = out[out.length - 1];
    if (!l) return true;
    // `</div>`: ein `/` nach `<` ist ein schließendes JSX-Tag, kein Regex.
    if (l.t === "p") return !")]}<".includes(l.v);
    if (l.t === "id")
      return /^(return|typeof|case|in|of|delete|void|throw)$/.test(l.v);
    return false;
  };
  /** Ende eines Strings mit Begrenzer `q` ab `j` (hinter dem öffnenden Zeichen). */
  const stringEnde = (j, q, mehrzeilig) => {
    for (; j < n; j++) {
      if (src[j] === "\\") j++;
      else if (src[j] === q) return j + 1;
      else if (!mehrzeilig && src[j] === "\n") break;
    }
    i = j;
    return fehler("nicht abgeschlossene Zeichenkette");
  };
  /** Template-Literal ab `j` (hinter dem öffnenden Backtick); liefert Ende und `${`-Befund. */
  const templateEnde = (j, ausdruecke = []) => {
    let interp = false;
    for (; j < n; j++) {
      if (src[j] === "\\") j++;
      else if (src[j] === "`") return [j + 1, interp];
      else if (src[j] === "$" && src[j + 1] === "{") {
        interp = true;
        const ende = ausdruckEnde(j + 2);
        ausdruecke.push([j + 2, ende]);
        j = ende;
      }
    }
    i = j;
    return fehler("nicht abgeschlossenes Template-Literal");
  };
  /** Ende eines `${ … }`-Ausdrucks (Index der schließenden Klammer), Zeichenketten beachtet. */
  const ausdruckEnde = (j) => {
    let tiefe = 1;
    for (; j < n; j++) {
      const c = src[j];
      if (c === '"' || c === "'") j = stringEnde(j + 1, c, false) - 1;
      else if (c === "`") j = templateEnde(j + 1)[0] - 1;
      else if (c === "{") tiefe++;
      else if (c === "}" && --tiefe === 0) return j;
    }
    i = j;
    return fehler("nicht abgeschlossener ${}-Ausdruck");
  };

  while (i < n) {
    const c = src[i];
    if (/\s/.test(c)) {
      i++;
      continue;
    }
    if (c === "/" && src[i + 1] === "/") {
      while (i < n && src[i] !== "\n") i++;
      continue;
    }
    if (c === "/" && src[i + 1] === "*") {
      let tiefe = 1;
      i += 2;
      while (i < n && tiefe > 0) {
        if (rust && src[i] === "/" && src[i + 1] === "*") (tiefe++, (i += 2));
        else if (src[i] === "*" && src[i + 1] === "/") (tiefe--, (i += 2));
        else i++;
      }
      if (tiefe > 0) fehler("nicht abgeschlossener Kommentar");
      continue;
    }
    // Rust: Raw-Strings r"…", r#"…"#, br#"…"#
    if (rust) {
      const m = /^b?r(#*)"/.exec(src.slice(i, i + 40));
      if (m) {
        const ende = `"${m[1]}`;
        const j = src.indexOf(ende, i + m[0].length);
        if (j < 0) fehler("nicht abgeschlossener Raw-String");
        out.push({ t: "str", v: src.slice(i, j + ende.length) });
        i = j + ende.length;
        continue;
      }
      if (c === "b" && src[i + 1] === '"') i++;
      if (src[i] === '"') {
        const j = stringEnde(i + 1, '"', true);
        out.push({ t: "str", v: src.slice(i, j) });
        i = j;
        continue;
      }
      // Zeichenliteral 'x', '\n', '\u{..}' – sonst ein Lebensdauer-Apostroph.
      if (src[i] === "'") {
        const m =
          /^'(?:\\u\{[0-9a-fA-F_]+\}|\\x[0-9a-fA-F]{2}|\\.|[^\\'])'/.exec(
            src.slice(i, i + 14),
          );
        if (m) {
          out.push({ t: "str", v: m[0] });
          i += m[0].length;
        } else {
          out.push({ t: "p", v: "'" });
          i++;
        }
        continue;
      }
    } else {
      if (c === '"' || c === "'") {
        const j = stringEnde(i + 1, c, false);
        out.push({ t: "str", v: src.slice(i, j) });
        i = j;
        continue;
      }
      if (c === "`") {
        const ausdruecke = [];
        const [j, interp] = templateEnde(i + 1, ausdruecke);
        out.push({ t: "str", v: src.slice(i, j), interp });
        // Die Ausdrücke in `${ … }` sind Code: eigene Token, in geschweifte Klammern gefasst.
        for (const [a, b] of ausdruecke)
          out.push({ t: "p", v: "{" }, ...tokenize(src.slice(a, b), sprache), {
            t: "p",
            v: "}",
          });
        i = j;
        continue;
      }
      if (c === "/" && regexMoeglich()) {
        let j = i + 1;
        let klasse = false;
        for (; j < n && src[j] !== "\n"; j++) {
          if (src[j] === "\\") j++;
          else if (src[j] === "[") klasse = true;
          else if (src[j] === "]") klasse = false;
          else if (src[j] === "/" && !klasse) break;
        }
        if (src[j] !== "/") fehler("nicht abgeschlossener Regex-Ausdruck");
        j++;
        while (j < n && /[a-z]/i.test(src[j])) j++;
        out.push({ t: "str", v: src.slice(i, j), regex: true });
        i = j;
        continue;
      }
    }
    if (IDSTART.test(c)) {
      let j = i + 1;
      while (j < n && IDREST.test(src[j])) j++;
      out.push({ t: "id", v: src.slice(i, j) });
      i = j;
      continue;
    }
    if (/[0-9]/.test(c)) {
      let j = i + 1;
      while (j < n && /[0-9a-zA-Z_.]/.test(src[j])) j++;
      out.push({ t: "str", v: src.slice(i, j), zahl: true });
      i = j;
      continue;
    }
    out.push({ t: "p", v: c });
    i++;
  }
  return out;
}

const ZU = { "(": ")", "[": "]", "{": "}" };

/** Index der Klammer, die die öffnende bei `von` schließt; sonst `Unsicher`. */
function schliessende(tok, von) {
  const stapel = [];
  for (let k = von; k < tok.length; k++) {
    const t = tok[k];
    if (t.t !== "p") continue;
    if (ZU[t.v]) stapel.push(ZU[t.v]);
    else if (")]}".includes(t.v)) {
      if (stapel.pop() !== t.v)
        throw new Unsicher(`Klammern passen nicht (Token ${k})`);
      if (stapel.length === 0) return k;
    }
  }
  throw new Unsicher(`Klammer ab Token ${von} bleibt offen`);
}

/**
 * Ist der Inhalt eines Attributs EXAKT eine Testmarke? Nur `test`, `cfg(test)` und Pfade, die auf
 * `::test` enden (`tokio::test`). Alles andere, auch `cfg(any(test, unix))`, `cfg(all(…))`, `cfg_attr`,
 * `cfg(not(test))`, ist keine Testmarke und blendet nichts aus.
 */
function istTestMarke(inhalt) {
  const s = inhalt.map((t) => t.v).join(" ");
  return (
    s === "test" ||
    s === "cfg ( test )" ||
    /^(?:[\p{L}_][\p{L}\p{N}_]* : : )+test$/u.test(s)
  );
}

/**
 * Rust: Token-Bereiche `[von, bis)`, die zu einer Testmarke gehören. Ein äußeres Attribut blendet den
 * zugehörigen Eintrag aus (`{ … }` per Klammertiefe oder bis `;`). Ein inneres `#![cfg(test)]` blendet
 * höchstens den umschließenden `{ … }`-Block aus; steht es auf oberster Ebene, blendet es NICHTS aus
 * (fail-closed: der Code wird als Produktionscode geprüft). Nicht bestimmbare Grenzen sind Fehler.
 */
function testBereiche(tok) {
  const bereiche = [];
  const offene = [];
  for (let k = 0; k < tok.length; k++) {
    if (tok[k].t === "p" && tok[k].v === "{") offene.push(k);
    if (tok[k].t === "p" && tok[k].v === "}") offene.pop();
    if (tok[k].v !== "#" || tok[k].t !== "p") continue;
    let a = k + 1;
    const innen = tok[a]?.v === "!";
    if (innen) a++;
    if (tok[a]?.v !== "[") continue;
    const zu = schliessende(tok, a);
    if (!istTestMarke(tok.slice(a + 1, zu))) {
      k = zu;
      continue;
    }
    if (innen) {
      const auf = offene[offene.length - 1];
      if (auf !== undefined) bereiche.push([auf, schliessende(tok, auf) + 1]);
      k = zu;
      continue;
    }
    // Der zugehörige Eintrag: weitere Attribute überspringen, dann bis `{ … }` oder `;`.
    let e = zu + 1;
    while (tok[e]?.v === "#" && tok[e + 1]?.v === "[")
      e = schliessende(tok, e + 1) + 1;
    let ende = -1;
    for (; e < tok.length; e++) {
      const t = tok[e];
      if (t.t !== "p") continue;
      if (t.v === ";") {
        ende = e + 1;
        break;
      }
      if (t.v === "{") {
        ende = schliessende(tok, e) + 1;
        break;
      }
      if (t.v === "(" || t.v === "[") e = schliessende(tok, e);
    }
    if (ende < 0)
      throw new Unsicher(
        `Blockgrenze des Testattributs ab Token ${k} nicht bestimmbar`,
      );
    bereiche.push([k, ende]);
    k = ende - 1;
  }
  return bereiche;
}

/** Textform eines Aufrufs für Meldung und Erlaubnisliste: Token mit einem Leerzeichen getrennt. */
const aufrufText = (tok, von, bis) =>
  tok
    .slice(von, bis + 1)
    .map((t) => t.v)
    .join(" ")
    .replace(/: :/g, "::");

/** Alle Argumente reine Textliterale (Rust: ohne `{}`; TypeScript: ohne `${}`)? */
function nurFesterText(args, rust) {
  if (args.length === 0) return false;
  let erwartet = "str";
  for (const t of args) {
    if (erwartet === "str") {
      if (t.t !== "str" || t.zahl || t.regex || t.interp) return false;
      if (rust && /[{}]/.test(t.v)) return false;
      if (!rust && /\$\{/.test(t.v)) return false;
      erwartet = rust ? "ende" : ",";
    } else if (!rust && t.t === "p" && t.v === ",") erwartet = "str";
    else return false;
  }
  return true;
}

/** Namen, die einen Import oder eine Makrodefinition als Log-Bezug ausweisen. */
const LOG_BEZUG = new Set([...RUST_LOGS, "log", "tracing", "slog"]);

/**
 * Prüft eine Quelle. Liefert `verstoesse` (Aufrufe, die weder fester Text noch erlaubt sind),
 * `genutzt` (Schlüssel der Erlaubnisliste, je Fundstelle einer) und `fehler` (nicht sicher
 * einordbare Konstrukte: unbekannte Literale, Blockgrenzen, Umbenennungen und Makros von Log-Makros).
 */
function pruefe(src, sprache, datei, erlaubt = ERLAUBT) {
  const rust = sprache === "rust";
  const ergebnis = { verstoesse: [], genutzt: [], fehler: [], aufrufe: 0 };
  let tok;
  let uebersprungen = [];
  try {
    tok = tokenize(src, sprache);
    if (rust) uebersprungen = testBereiche(tok);
  } catch (e) {
    if (!(e instanceof Unsicher)) throw e;
    ergebnis.fehler.push(`${datei}: ${e.message}`);
    return ergebnis;
  }
  const inTest = (k) => uebersprungen.some(([a, b]) => k >= a && k < b);
  const melde = (k, bis, args) => {
    ergebnis.aufrufe++;
    if (nurFesterText(args, rust)) return;
    const text = aufrufText(tok, k, bis);
    if (erlaubt.has(`${datei}|${text}`))
      ergebnis.genutzt.push(`${datei}|${text}`);
    else ergebnis.verstoesse.push(`${datei}: ${text.slice(0, 140)}`);
  };
  /** Rust: Anfang des Aufrufpfads (`log :: warn !`): rückwärts über `name ::`. */
  const pfadAnfang = (k) => {
    while (
      tok[k - 1]?.v === ":" &&
      tok[k - 2]?.v === ":" &&
      tok[k - 3]?.t === "id"
    )
      k -= 3;
    return k;
  };
  try {
    for (let k = 0; k < tok.length; k++) {
      const t = tok[k];
      if (t.t !== "id" || inTest(k)) continue;
      if (rust) {
        // Umbenennung eines Log-Makros per `use … as`: der Aufruf ließe sich nicht mehr erkennen.
        if (t.v === "use" || (t.v === "extern" && tok[k + 1]?.v === "crate")) {
          let e = k + 1;
          for (; e < tok.length && tok[e].v !== ";"; e++)
            if (tok[e].t === "p" && tok[e].v === "{") e = schliessende(tok, e);
          const teil = tok.slice(k, e);
          if (
            teil.some((x) => x.t === "id" && x.v === "as") &&
            teil.some((x) => x.t === "id" && LOG_BEZUG.has(x.v))
          )
            ergebnis.fehler.push(
              `${datei}: nicht prüfbar, Umbenennung eines Log-Imports: ${aufrufText(tok, k, e)}`,
            );
          continue;
        }
        // Eigene Makros, deren Rumpf ein Log-Makro ruft, verstecken den Aufruf.
        if (t.v === "macro_rules" && tok[k + 1]?.v === "!") {
          const auf = tok.findIndex(
            (x, i) => i > k + 1 && x.t === "p" && ZU[x.v],
          );
          if (auf < 0) throw new Unsicher("macro_rules ohne Rumpf");
          const zu = schliessende(tok, auf);
          for (let i = auf; i < zu; i++)
            if (
              tok[i].t === "id" &&
              RUST_LOGS.has(tok[i].v) &&
              tok[i + 1]?.v === "!"
            ) {
              ergebnis.fehler.push(
                `${datei}: nicht prüfbar, macro_rules ruft ein Log-Makro: ${tok[k + 2]?.v}`,
              );
              break;
            }
          k = zu;
          continue;
        }
        // `name ! (`; der Pfad davor (`log ::`) gehört zum Aufruf.
        if (!RUST_LOGS.has(t.v) || tok[k + 1]?.v !== "!") continue;
        const auf = tok[k + 2];
        if (auf?.t !== "p" || !ZU[auf.v]) continue;
        const zu = schliessende(tok, k + 2);
        melde(pfadAnfang(k), zu, tok.slice(k + 3, zu));
      } else {
        if (t.v !== "console") continue;
        let a = k + 1;
        if (tok[a]?.v === "?") a++;
        if (tok[a]?.v !== "." || tok[a + 1]?.t !== "id") {
          // `console` ohne anschließenden Aufruf (Alias, `console["log"]`): nicht prüfbar.
          ergebnis.verstoesse.push(
            `${datei}: console ohne unmittelbaren Aufruf: ${aufrufText(tok, k, Math.min(k + 4, tok.length - 1))}`,
          );
          continue;
        }
        const name = tok[a + 1].v;
        if (!TS_LOGS.has(name)) continue;
        if (tok[a + 2]?.v !== "(") {
          ergebnis.verstoesse.push(
            `${datei}: console.${name} ohne unmittelbaren Aufruf (Alias)`,
          );
          continue;
        }
        const zu = schliessende(tok, a + 2);
        melde(k, zu, tok.slice(a + 3, zu));
      }
    }
  } catch (e) {
    if (!(e instanceof Unsicher)) throw e;
    ergebnis.fehler.push(`${datei}: ${e.message}`);
  }
  return ergebnis;
}

const lies = (rel) =>
  readFileSync(join(wurzel, rel), "utf8").replace(/\r\n/g, "\n");

// ---------------------------------------------------------------------------------------------
// Prüfungen
// ---------------------------------------------------------------------------------------------

test("Log-Hygiene: die Dateilisten sind nicht leer", () => {
  assert.ok(RUST.length >= 8, `Rust-Dateien: ${RUST.length}`);
  assert.ok(TS.length >= 10, `TypeScript-Dateien: ${TS.length}`);
});

for (const [liste, sprache] of [
  [RUST, "rust"],
  [TS, "ts"],
]) {
  for (const rel of liste) {
    test(`Log-Hygiene ${sprache === "rust" ? "Rust" : "TypeScript"}: ${rel} loggt nur feste Texte`, () => {
      const r = pruefe(lies(rel), sprache, rel);
      assert.deepEqual(r.fehler, [], "Quelltext nicht sicher lesbar");
      assert.deepEqual(
        r.verstoesse,
        [],
        "Log-Aufruf mit Platzhalter oder Argument",
      );
    });
  }
}

test("Log-Hygiene: jede Ausnahme hat genau eine Fundstelle", () => {
  const funde = new Map();
  for (const [liste, sprache] of [
    [RUST, "rust"],
    [TS, "ts"],
  ])
    for (const rel of liste)
      for (const k of pruefe(lies(rel), sprache, rel).genutzt)
        funde.set(k, (funde.get(k) ?? 0) + 1);
  for (const k of ERLAUBT.keys())
    assert.equal(
      funde.get(k) ?? 0,
      1,
      `Ausnahme ohne genau eine Fundstelle: ${k}`,
    );
});

test("Log-Hygiene: die Erlaubnisliste enthält nichts mit Inhaltsbezug", () => {
  for (const schluessel of ERLAUBT.keys()) {
    const aufruf = schluessel.slice(schluessel.indexOf("|") + 1);
    assert.equal(INHALT.test(aufruf), false, aufruf);
  }
});

// ---------------------------------------------------------------------------------------------
// Selbsttest: dieselbe Prüfung muss die bekannten Gegenbeispiele erkennen
// ---------------------------------------------------------------------------------------------

const rs = (q, erlaubt = new Map(), datei = "a.rs") =>
  pruefe(q, "rust", datei, erlaubt);
const ts = (q, erlaubt = new Map(), datei = "a.ts") =>
  pruefe(q, "ts", datei, erlaubt);
const zahl = (r) => r.verstoesse.length;

test("Selbsttest: Verstöße mit Leerraum, Zeilenumbruch und mehrzeiligen Aufrufen fallen auf", () => {
  assert.equal(zahl(rs('fn f() { log::info!("Auszug: {}", excerpt); }')), 1);
  assert.equal(zahl(rs('fn f() { log::info! ("{}", excerpt); }')), 1);
  assert.equal(zahl(rs('fn f() { log :: info !\n(\n"{}",\nexcerpt\n); }')), 1);
  assert.equal(zahl(rs('fn f() { log::warn!("{text}"); }')), 1);
  assert.equal(zahl(rs("fn f() { println !(payload); }")), 1);
  assert.equal(zahl(rs('fn f() { log::debug!("Abruf", payload); }')), 1);
  assert.equal(zahl(ts("console.log(payload);")), 1);
  assert.equal(zahl(ts("console.log (payload);")), 1);
  assert.equal(zahl(ts("console\n  .log\n  (\n payload\n );")), 1);
  assert.equal(zahl(ts("console . log(payload);")), 1);
  assert.equal(zahl(ts("console?.log(payload);")), 1);
  assert.equal(zahl(ts('console.log("Treffer", hits);')), 1);
  assert.equal(zahl(ts("console.warn(`Frage ${frage}`);")), 1);
  assert.equal(zahl(ts("const l = console.log; l(payload);")), 1);
  assert.equal(zahl(ts('console["log"](payload);')), 1);
});

test("Selbsttest: feste Texte, Kommentare und Zeichenketten sind kein Verstoß", () => {
  assert.equal(zahl(rs('log::info!("Abruf verworfen");')), 0);
  assert.equal(rs('log::info!("Abruf verworfen");').aufrufe, 1);
  assert.equal(zahl(rs('log::info!(r#"fester "Text""#);')), 0);
  assert.equal(
    zahl(rs('fn f() { // log::info!("{x}", y);\n /* println!("{x}") */ }')),
    0,
  );
  assert.equal(
    zahl(rs('fn f() { let s = "log::info!(\\"{x}\\", y)"; let c = \'"\'; }')),
    0,
  );
  assert.equal(zahl(rs("fn f<'a>(x: &'a str) { log::info!(\"fest\"); }")), 0);
  assert.equal(zahl(ts('console.error("Start fehlgeschlagen");')), 0);
  assert.equal(zahl(ts('console.error("Start", "fehlgeschlagen");')), 0);
  assert.equal(zahl(ts("console.warn(`fester Text`);")), 0);
  assert.equal(
    zahl(ts('// console.log(payload)\nconst s = "console.log(payload)";')),
    0,
  );
  assert.equal(zahl(ts('const re = /"/g; console.log("fest");')), 0);
});

test("Selbsttest: Testblöcke werden übersprungen, Produktionscode danach wird geprüft", () => {
  const nurTest =
    'fn f() {}\n#[cfg(test)]\nmod t { fn g() { println!("{x}"); } }';
  assert.equal(zahl(rs(nurTest)), 0);
  const danach = `${nurTest}\nfn prod() { info!("{text}"); }`;
  assert.deepEqual(rs(danach).verstoesse.length, 1);
  assert.equal(rs(danach).aufrufe, 1, "der Aufruf im Testblock zählt nicht");
  const einzeln =
    '#[test]\nfn t() { println!("{x}"); }\nfn p() { warn!("{y}"); }';
  assert.equal(zahl(rs(einzeln)), 1);
  const verschachtelt =
    '#[cfg(test)]\nmod t { mod u { fn g() { println!("{x}"); } } fn h() { println!("{y}"); } }\nfn p() { error!("{z}"); }';
  assert.equal(zahl(rs(verschachtelt)), 1);
  const klammerInString =
    '#[cfg(test)]\nmod t { fn g() { let s = "}"; println!("{x}"); } }\nfn p() { error!("{z}"); }';
  assert.equal(zahl(rs(klammerInString)), 1);
  const mehrereAttribute =
    '#[cfg(test)]\n#[allow(unused)]\nmod t { fn g() { println!("{x}"); } }\nfn p() { info!("{z}"); }';
  assert.equal(zahl(rs(mehrereAttribute)), 1);
  const ohneBlock = '#[cfg(test)]\nmod tests;\nfn p() { info!("{z}"); }';
  assert.equal(zahl(rs(ohneBlock)), 1);
  const nichtTest = '#[cfg(not(test))]\nfn p() { info!("{z}"); }';
  assert.equal(zahl(rs(nichtTest)), 1, "`not(test)` ist Produktionscode");
});

test("Selbsttest: unsichere Blockgrenzen und Literale werden als Fehler gemeldet", () => {
  assert.equal(
    rs('#[cfg(test)]\nmod t { fn g() { println!("x"); }').fehler.length,
    1,
  );
  assert.equal(rs("#[cfg(test)]\n").fehler.length, 1);
  assert.equal(rs('fn f() { let s = "offen; }').fehler.length, 1);
  assert.equal(ts('console.log("offen);').fehler.length, 1);
  assert.equal(rs("/* offen").fehler.length, 1);
});

test("Selbsttest: Template-Interpolation wird als Code geprüft", () => {
  assert.equal(zahl(ts("const s = `${console.log(payload)}`;")), 1);
  assert.equal(zahl(ts("const s = `a ${`b ${console.log(payload)}`} c`;")), 1);
  assert.equal(zahl(ts('const s = `${"}"} ${console . warn (x)}`;')), 1);
  assert.equal(zahl(ts("const s = `${console.log(`fest`)}`;")), 0);
  assert.equal(zahl(ts("const s = `${a + b}`;")), 0);
});

test("Selbsttest: nur exakte Testmarken blenden aus", () => {
  const prod = 'fn prod() { log::info!("{}", payload); }';
  for (const attr of [
    "#[cfg(any(test, unix))]",
    "#[cfg(all(test, unix))]",
    "#[cfg_attr(test, derive(Debug))]",
    "#[cfg(not(test))]",
    '#[cfg(feature = "test")]',
  ])
    assert.equal(zahl(rs(`${attr}\n${prod}`)), 1, attr);
  for (const attr of [
    "#[cfg(test)]",
    "#[test]",
    "#[tokio::test]",
    "#[a::b::test]",
  ])
    assert.equal(zahl(rs(`${attr}\n${prod}`)), 0, attr);
  // Inneres Attribut: höchstens der umschließende Block, Code danach wird geprüft.
  assert.equal(zahl(rs(`mod tests { #![cfg(test)] }\n${prod}`)), 1);
  assert.equal(
    zahl(
      rs(`mod tests { #![cfg(test)] fn g() { println!("{x}"); } }\n${prod}`),
    ),
    1,
    "nur der Block ausgeblendet, `prod` danach geprüft",
  );
  assert.equal(
    zahl(rs(`#![cfg(test)]\n${prod}`)),
    1,
    "oberste Ebene: nichts ausgeblendet",
  );
});

test("Selbsttest: Makro-Alias und eigene Makros sind nicht prüfbar", () => {
  const alias = rs(
    'use log::info as audit; fn prod() { audit!("{}", payload); }',
  );
  assert.equal(alias.fehler.length, 1);
  assert.equal(rs("use log::{warn as w, debug};").fehler.length, 1);
  assert.equal(rs("use log as l; fn f() {}").fehler.length, 1);
  assert.equal(rs("use tracing::error as fehler;").fehler.length, 1);
  assert.equal(
    rs('macro_rules! m { ($x:expr) => { log::info!("{}", $x) }; }').fehler
      .length,
    1,
  );
  // Gewöhnliche Umbenennungen bleiben möglich.
  assert.equal(
    rs("use base64::Engine as _; use a::b::{c as d};").fehler.length,
    0,
  );
  assert.equal(rs("macro_rules! m { () => { 1 + 1 }; }").fehler.length, 0);
});

test("Selbsttest: der Aufrufpfad gehört zum Schlüssel der Erlaubnisliste", () => {
  const text = '"voice_tools: {what} nicht lesbar: {e}"';
  const erlaubt = new Map([[`a.rs|log :: warn ! ( ${text} )`, "Test"]]);
  assert.equal(zahl(rs(`fn f() { log::warn!(${text}); }`, erlaubt)), 0);
  assert.equal(
    rs(`fn f() { log::warn!(${text}); }`, erlaubt).genutzt.length,
    1,
  );
  assert.equal(zahl(rs(`fn f() { other::warn!(${text}); }`, erlaubt)), 1);
  assert.equal(zahl(rs(`fn f() { warn!(${text}); }`, erlaubt)), 1);
  assert.equal(zahl(rs(`fn f() { log::error!(${text}); }`, erlaubt)), 1);
});

test("Selbsttest: Ausnahmen gelten nur für Datei und Aufruf, höchstens einmal", () => {
  const aufruf = 'console.warn("story send failed", e);';
  const erlaubt = new Map([
    ['a.ts|console . warn ( "story send failed" , e )', "Test"],
  ]);
  const einmal = ts(aufruf, erlaubt, "a.ts");
  assert.equal(zahl(einmal), 0);
  assert.deepEqual(einmal.genutzt, [
    'a.ts|console . warn ( "story send failed" , e )',
  ]);
  // Zweites Vorkommen: es gibt zwei Fundstellen, die Prüfung der Fundstellen (genau eine) schlägt an.
  const zweimal = ts(`${aufruf}\n${aufruf}`, erlaubt, "a.ts");
  assert.equal(zweimal.genutzt.length, 2);
  // Derselbe Text in einer anderen Datei besteht nicht.
  assert.equal(zahl(ts(aufruf, erlaubt, "b.ts")), 1);
  // Ein anderer Aufruf in derselben Datei besteht nicht.
  assert.equal(
    zahl(ts('console.warn("story send failed", payload);', erlaubt, "a.ts")),
    1,
  );
  assert.equal(
    zahl(ts('console.error("story send failed", e);', erlaubt, "a.ts")),
    1,
  );
});
