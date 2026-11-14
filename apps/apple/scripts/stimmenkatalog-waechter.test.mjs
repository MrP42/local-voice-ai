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

async function swiftSources(directory, relative = '', settingsLinks = false) {
    const sources = [];
    for (const entry of await readdir(directory, { withFileTypes: true })) {
        const name = relative ? `${relative}/${entry.name}` : entry.name;
        const absolute = path.join(directory, entry.name);
        if (entry.isDirectory()) {
            sources.push(...await swiftSources(absolute, name, settingsLinks));
        } else if (entry.isFile() && (settingsLinks || entry.name.endsWith('.swift'))) {
            const data = await readFile(absolute);
            if (!settingsLinks || data.includes(settingsScheme)) {
                sources.push({ name, text: data.toString('utf8') });
            }
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

// Zusammengesetzt, damit der Wächter nicht sein eigenes Prüfmuster als Fund zählt.
const settingsScheme = 'App' + '-Prefs';
const settingsFlag = 'LV_DIRECT_SETTINGS_LINK';

function unguardedSettingsLinks(sources) {
    return sources.flatMap(({ name, text }) => {
        const blocks = [];
        const violations = [];
        for (const [index, line] of text.split('\n').entries()) {
            const directive = line.trim();
            if (/^#if\s/.test(directive)) {
                blocks.push(directive === `#if ${settingsFlag}`);
            } else if (/^#(?:else|elseif)\b/.test(directive)) {
                // Der Gegenast des geschützten Blocks wird auch im Release übersetzt.
                if (blocks.length) blocks[blocks.length - 1] = false;
            } else if (/^#endif\b/.test(directive)) {
                blocks.pop();
            }
            if (line.includes(settingsScheme)
                && (!name.endsWith('.swift') || !blocks.includes(true))) {
                violations.push(`${name}:${index + 1}`);
            }
        }
        return violations;
    });
}

test('Private Einstellungslinks stehen nur in Swift unter dem Eigenbau-Flag', async () => {
    assert.deepEqual(unguardedSettingsLinks(await swiftSources(appleRoot, '', true)), []);
});

test('Gegenprobe: ungeschützte Links, Gegenäste und andere Dateitypen fallen auf', () => {
    for (const text of [
        settingsScheme,
        `#if DEBUG\n${settingsScheme}\n#endif`,
        `#if ${settingsFlag}\n#else\n${settingsScheme}\n#endif`,
        `#if ${settingsFlag}\n#elseif DEBUG\n${settingsScheme}\n#endif`,
        `#if ${settingsFlag}\n#endif\n${settingsScheme}`,
    ]) {
        assert.notDeepEqual(unguardedSettingsLinks([{ name: 'Probe.swift', text }]), []);
    }
    const text = `#if ${settingsFlag}\n#if DEBUG\n${settingsScheme}\n#endif\n#endif`;
    assert.deepEqual(unguardedSettingsLinks([{ name: 'Probe.swift', text }]), []);
    assert.notDeepEqual(unguardedSettingsLinks([{ name: 'Probe.plist', text }]), []);
});

function debugOnlySettingsFlag(text) {
    const branches = text.split('\n').filter(line => line.includes("mode == 'Debug' else"));
    return branches.length === 1
        && branches[0].trim() === "debug if mode == 'Debug' else "
            + "{'SWIFT_OPTIMIZATION_LEVEL': '-O'}"
        && text.includes("'DEBUG LV_DIRECT_SETTINGS_LINK' if settings.get('SDKROOT') "
            + "== 'iphoneos' else 'DEBUG'");
}

test('Der Projektgenerator setzt das Link-Flag ausschließlich in iPhone-Debug', async () => {
    const text = await readFile(path.join(appleRoot, 'scripts/generate_project.py'), 'utf8');
    assert.ok(debugOnlySettingsFlag(text));
    assert.equal(debugOnlySettingsFlag(text.replace(
        "else {'SWIFT_OPTIMIZATION_LEVEL': '-O'}",
        "else {'SWIFT_ACTIVE_COMPILATION_CONDITIONS': 'LV_DIRECT_SETTINGS_LINK'}",
    )), false);
    assert.equal(debugOnlySettingsFlag(text.replace(
        'DEBUG LV_DIRECT_SETTINGS_LINK', 'DEBUG',
    )), false);
});

function protectsCurrentChoice(text) {
    const fallback = text.slice(text.indexOf('private func fallbackVoice('));
    const own = fallback.indexOf(
        'if hasOwnChoice, let id = voiceIdentifier,',
    );
    return own >= 0 && own < fallback.indexOf('if let automatic, automatic.quality')
        && text.includes('hasBook || hasLanguage || ownCurrent')
        && text.includes('markVoiceChosenByUser(false)')
        && text.includes('if let lang = voiceLanguage, hasCurrentVoiceChoice(lang)');
}

test('Passende eigene Stimmen werden vor der automatischen Verbesserung geschützt', async () => {
    const text = await readFile(
        path.join(appleRoot, 'iPhone/Library/LibraryPlayback.swift'), 'utf8',
    );
    assert.ok(protectsCurrentChoice(text));
    assert.equal(protectsCurrentChoice(text.replace(
        'if hasOwnChoice, let id = voiceIdentifier,', '',
    )), false);
    assert.equal(protectsCurrentChoice(text.replace(' || ownCurrent', '')), false);
    assert.equal(protectsCurrentChoice(text.replace('markVoiceChosenByUser(false)', '')), false);
});

test('Der globale Picker markiert auch Phonon und Systemstandard als Tipp-Auswahl', async () => {
    const text = await readFile(path.join(appleRoot, 'Shared/SpeechVoicePicker.swift'), 'utf8');
    const wired = source => ['selectVoice("")', 'selectVoice(id)', 'selectVoice(voice.id)',
        'LibraryPlayback.shared.chooseGlobalVoice(id)'].every(part => source.includes(part));
    assert.ok(wired(text));
    assert.equal(wired(text.replace('selectVoice(id)', 'model.selectedVoiceID = id')), false);
    assert.equal(wired(text.replace('LibraryPlayback.shared.chooseGlobalVoice(id)', '')), false);
});

function hasSettingsFallback(text) {
    const speech = text.indexOf(`${settingsScheme}:root=ACCESSIBILITY&path=SPEECH_TITLE`);
    const accessibility = text.indexOf(`"${settingsScheme}:root=ACCESSIBILITY"`);
    const app = text.indexOf('UIApplication.openSettingsURLString');
    return speech >= 0 && speech < accessibility && accessibility < app
        && text.includes('UIApplication.shared.canOpenURL(url) else {\n'
            + '            open(remaining)')
        && text.includes('UIApplication.shared.open(url, options: [:]) { success in\n'
            + '            if !success {\n'
            + '                Task { @MainActor in open(remaining) }');
}

test('Der direkte Link versucht bei jeder Ablehnung die nächste Stufe', async () => {
    const text = await readFile(path.join(appleRoot, catalogPath), 'utf8');
    assert.ok(hasSettingsFallback(text));
    assert.equal(hasSettingsFallback(text.replace('open(remaining)', 'return')), false);
    assert.equal(hasSettingsFallback(text.replace('if !success {', 'if success {')), false);
});

test('Die drei Tipp-Auswahlen im Picker haben mindestens 44 pt Höhe', async () => {
    const text = await readFile(path.join(appleRoot, 'Shared/SpeechVoicePicker.swift'), 'utf8');
    const picker = text.split('struct MicrophoneSettings: View')[0];
    const target = '.frame(minHeight: 44).contentShape(Rectangle())';
    const hasTargets = source => source.split(target).length - 1 === 3;
    assert.ok(hasTargets(picker));
    assert.equal(hasTargets(picker.replace(target, '.contentShape(Rectangle())')), false);
});
