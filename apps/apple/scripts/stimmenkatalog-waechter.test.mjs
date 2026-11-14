import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const appleRoot = fileURLToPath(new URL('../', import.meta.url));
const catalogPath = 'iPhone/Library/VoiceCatalog.swift';
const macPath = 'Shared/VoiceModel.swift'; // nur der macOS-Zweig (#else), iOS nutzt den Katalog
const erlaubt = [catalogPath, macPath].sort();
const enumeration = 'speechVoices()';

async function swiftSources(directory, relative = '') {
    const sources = [];
    for (const entry of await readdir(directory, { withFileTypes: true })) {
        const name = relative ? `${relative}/${entry.name}` : entry.name;
        const absolute = path.join(directory, entry.name);
        if (entry.isDirectory()) sources.push(...await swiftSources(absolute, name));
        else if (entry.isFile() && entry.name.endsWith('.swift')) {
            sources.push({ name, text: await readFile(absolute, 'utf8') });
        }
    }
    return sources;
}

function enumerationSites(sources) {
    return sources.flatMap(({ name, text }) =>
        Array.from({ length: text.split(enumeration).length - 1 }, () => name)).sort();
}

test('Nur Katalog (iOS) und macOS-Zweig fragen den Systemkatalog je einmal ab', async () => {
    assert.deepEqual(enumerationSites(await swiftSources(appleRoot)), erlaubt);
});

test('Gegenprobe: ein Aufruf in einer Bibliotheksdatei wird erkannt', () => {
    const sources = [
        { name: catalogPath, text: enumeration },
        { name: macPath, text: enumeration },
        { name: 'iPhone/Library/LibraryPlayback.swift', text: enumeration },
    ];
    assert.notDeepEqual(enumerationSites(sources), erlaubt);
});

test('Gegenprobe: auch eine zweite Abfrage im Katalog wird erkannt', () => {
    const sources = [{ name: catalogPath, text: `${enumeration}\n${enumeration}` }];
    assert.notDeepEqual(enumerationSites(sources), [catalogPath]);
});
