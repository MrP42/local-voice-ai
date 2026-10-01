#!/usr/bin/env node
// Prueft eine gebaute Windows-EXE auf das, was den Absturz in 0.21.0 verursachte (#70, #73):
//   1. Der Haupt-Thread (Webview, Command-Aufruf) braucht Stack: SizeOfStackReserve im PE-Kopf
//      muss mindestens --min-reserve-mib (Standard 8) betragen. Windows gibt sonst 1 MiB, und
//      Commands mit grossem Future (Rahmen von 400 KiB bis 1,4 MiB) laufen beim Klick in
//      STATUS_STACK_OVERFLOW (0xc00000fd), ohne Logzeile.
//   2. Kein Stack-Rahmen darf einen Bruchteil dieser Reserve ueberschreiten: Funktionen mit
//      `mov eax, <Groesse>; call __chkstk` ab --warn-frame-kib (Standard 256) werden aufgelistet,
//      ab einem Viertel der Reserve ist es ein Fehler.
//
// Aufruf: node scripts/check-exe-stack.mjs <pfad\zur\local-voice-ai.exe> [--min-reserve-mib 8]
// Exit 0 in Ordnung, 1 Reserve zu klein, 2 Rahmen zu gross, 3 keine lesbare PE-Datei.
// Reine Byte-Auswertung (.pdata + __chkstk-Muster), keine Zusatzwerkzeuge.

import { readFileSync } from "node:fs";

const args = process.argv.slice(2);
const file = args.find((a) => !a.startsWith("--"));
const option = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 ? Number(args[i + 1]) : fallback;
};
const minReserve = option("--min-reserve-mib", 8) * 1024 * 1024;
const warnFrame = option("--warn-frame-kib", 256) * 1024;

if (!file) {
  console.error(
    "Aufruf: node scripts/check-exe-stack.mjs <exe> [--min-reserve-mib N] [--warn-frame-kib N]",
  );
  process.exit(3);
}

function analyse(buffer) {
  const pe = buffer.readUInt32LE(0x3c);
  if (buffer.toString("latin1", pe, pe + 4) !== "PE\0\0")
    throw new Error("keine PE-Datei");
  const sectionCount = buffer.readUInt16LE(pe + 6);
  const optionalSize = buffer.readUInt16LE(pe + 20);
  const optional = pe + 24;
  if (buffer.readUInt16LE(optional) !== 0x20b)
    throw new Error("kein PE32+ (64 Bit)");
  const stackReserve = Number(buffer.readBigUInt64LE(optional + 72));
  const sections = [];
  let at = optional + optionalSize;
  for (let i = 0; i < sectionCount; i += 1, at += 40) {
    sections.push({
      name: buffer.toString("latin1", at, at + 8).replace(/\0+$/, ""),
      virtualSize: buffer.readUInt32LE(at + 8),
      va: buffer.readUInt32LE(at + 12),
      rawSize: buffer.readUInt32LE(at + 16),
      rawPtr: buffer.readUInt32LE(at + 20),
    });
  }
  const toOffset = (rva) => {
    for (const s of sections) {
      if (rva >= s.va && rva < s.va + Math.max(s.virtualSize, s.rawSize))
        return s.rawPtr + (rva - s.va);
    }
    return -1;
  };
  const exception = optional + 112 + 3 * 8;
  const exceptionRva = buffer.readUInt32LE(exception);
  const exceptionSize = buffer.readUInt32LE(exception + 4);
  const text = sections.find((s) => s.name === ".text");
  const frames = [];
  if (text && exceptionRva) {
    // __chkstk: sub rsp,10h / mov [rsp],r10 / mov [rsp+8],r11 / xor r11,r11 / lea r10,[rsp+18h] / sub r10,rax
    const signature = Buffer.from(
      "4883EC104C8914244C895C24084D33DB4C8D5424184C2BD0",
      "hex",
    );
    const found = buffer
      .subarray(text.rawPtr, text.rawPtr + text.rawSize)
      .indexOf(signature);
    if (found >= 0) {
      const chkstk = text.va + found;
      const table = toOffset(exceptionRva);
      for (let i = 0; i < Math.floor(exceptionSize / 12); i += 1) {
        const begin = buffer.readUInt32LE(table + i * 12);
        const end = buffer.readUInt32LE(table + i * 12 + 4);
        const offset = toOffset(begin);
        if (offset < 0) continue;
        const head = buffer.subarray(offset, offset + 96);
        for (let k = 0; k + 10 <= head.length; k += 1) {
          // mov eax, imm32 (B8) ; call rel32 (E8)
          if (head[k] === 0xb8 && head[k + 5] === 0xe8) {
            const size = head.readUInt32LE(k + 1);
            const target = begin + k + 10 + head.readInt32LE(k + 6);
            if (target === chkstk)
              frames.push({ rva: begin, bytes: size, codeBytes: end - begin });
            break;
          }
        }
      }
    }
  }
  frames.sort((a, b) => b.bytes - a.bytes);
  return { stackReserve, frames };
}

const kib = (n) => `${(n / 1024).toFixed(0)} KiB`;
let result;
try {
  result = analyse(readFileSync(file));
} catch (error) {
  console.error(`FEHLER: ${file}: ${error.message}`);
  process.exit(3);
}
const { stackReserve, frames } = result;
console.log(`${file}`);
console.log(
  `  Stack-Reserve des Haupt-Threads: ${kib(stackReserve)} (verlangt: mindestens ${kib(minReserve)})`,
);
const big = frames.filter((f) => f.bytes >= warnFrame);
console.log(
  `  Funktionen mit Stack-Rahmen ab ${kib(warnFrame)}: ${big.length}`,
);
for (const f of big.slice(0, 8)) {
  console.log(`    ${kib(f.bytes).padStart(9)}  rva 0x${f.rva.toString(16)}`);
}
let code = 0;
if (stackReserve < minReserve) {
  console.error(
    `FEHLER: Stack-Reserve ${kib(stackReserve)} < ${kib(minReserve)} (link-arg /STACK in build.rs fehlt?)`,
  );
  code = 1;
}
const limit = Math.floor(stackReserve / 4);
if (frames.length > 0 && frames[0].bytes > limit) {
  console.error(
    `FEHLER: Rahmen von ${kib(frames[0].bytes)} ist groesser als ein Viertel der Reserve (${kib(limit)})`,
  );
  code = code || 2;
}
if (code === 0) console.log("  OK");
process.exit(code);
