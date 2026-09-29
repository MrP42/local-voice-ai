"""Deterministische Dialog-Erzeugung fuer den synthetischen Mehrsprecher-Korpus.

Reine Logik ohne Ein-/Ausgabe (testbar): Texte entstehen aus Vorlagen mit Platzhaltern
(Fachbegriffe, englische Einsprengsel, Personen, Termine, Zahlen - alles ausgeschrieben,
keine Ziffern, damit der TTS-Text der Referenz entspricht). Dieselbe ``seed`` ergibt
dieselben Texte und dieselbe Zeitplanung.
"""
from __future__ import annotations

import random
import re
from dataclasses import dataclass, field

GENERATOR_VERSION = 1

_ARTICLES = {  # Genus -> (nom, akk, dat)
    "m": ("der", "den", "dem"),
    "f": ("die", "die", "der"),
    "n": ("das", "das", "dem"),
}

# ----------------------------------------------------------------------------- Vorlagen
# {A}/{B}/{C}: deutsche Fachbegriffe, {E}/{F}: englische Begriffe (Kasus: .nom/.akk/.dat),
# {P}: Person, {D}: Frist nach "bis", {T}: Zeitpunkt (eigenstaendig), {PC}: Prozent, {DU}: Dauer.
QUESTIONS = [
    "Wie weit sind wir eigentlich mit {A.dat}?",
    "Können wir {A.akk} bis {D} abschließen?",
    "Wer kümmert sich um {A.akk}, und wer hat {B.akk} im Blick?",
    "Haben wir für {E.akk} schon einen verbindlichen Termin?",
    "Was passiert, wenn {E.nom} nicht rechtzeitig fertig wird?",
    "Ist {A.nom} von {P} schon freigegeben worden?",
    "Reichen {DU} für {A.akk} und {E.akk} überhaupt aus?",
    "Wie hoch ist der Puffer für {A.akk}, nachdem wir {PC} eingeplant haben?",
    "Haben Sie {B.akk} schon mit {P} abgestimmt?",
    "Und wie sieht es bei {E.dat} aus, gibt es dort neue Risiken?",
]
ANSWERS = [
    "Ich habe {A.akk} gestern mit {P} durchgesprochen, wir liegen im Plan.",
    "Das hängt vor allem von {B.dat} ab; {P} meldet sich dazu {T}.",
    "Aktuell fehlen uns noch {DU}, dann können wir {E.akk} starten.",
    "Nach meiner Einschätzung schaffen wir {A.akk} {T}, wenn {P} rechtzeitig liefert.",
    "Wir haben {E.akk} bereits getestet, aber {B.nom} macht uns noch Sorgen.",
    "Bei {A.dat} liegen wir etwa {PC} über dem ursprünglichen Ansatz.",
    "Soweit ich weiß, ist {B.nom} fertig, nur {E.nom} steht noch aus.",
    "Das klären wir {T} mit {P}, dann bekommen Sie eine verbindliche Antwort.",
]
STATEMENTS = [
    "Wichtig ist mir, dass {A.nom} und {B.nom} zusammen betrachtet werden.",
    "Aus Sicht von {P} sollten wir {E.akk} vor {A.dat} priorisieren.",
    "Die Kosten für {A.akk} liegen derzeit bei {PC} über dem Plan.",
    "Ich schlage vor, dass wir {E.akk} {T} noch einmal gemeinsam durchgehen.",
    "Für den Kunden zählt am Ende vor allem, dass {A.nom} verlässlich funktioniert.",
    "Wir sollten {C.akk} nicht unterschätzen, denn {P} rechnet mit {DU} Mehraufwand.",
    "Kurz zur Lage: {A.nom} ist abgeschlossen, {B.nom} läuft, und {E.nom} beginnt {T}.",
    "Bis {D} brauchen wir eine Entscheidung zu {A.dat}, sonst verschiebt sich {E.nom}.",
]
AGREEMENTS = [
    "Einverstanden, dann übernehme ich {A.akk} und melde mich {T} bei {P}.",
    "Da habe ich Bedenken, weil {B.nom} noch nicht sauber dokumentiert ist.",
    "Das sehe ich anders: {E.nom} sollte erst nach {A.dat} kommen.",
    "Gut, dann halten wir fest: {P} kümmert sich um {A.akk}, ich um {B.akk}.",
    "Das klingt vernünftig, aber wir brauchen dafür mindestens {DU}.",
    "Ja, genau so machen wir das, und {E.akk} nehmen wir ins Protokoll auf.",
]
OPENERS = [
    "Guten Tag zusammen, wir beginnen heute mit {A.dat} und schauen danach auf {E.akk}.",
    "Schön, dass alle da sind. Heute geht es um {A.akk}, {B.akk} und {E.akk}.",
]
CLOSERS = [
    "Dann bedanke ich mich bei allen, {P} verschickt das Protokoll, und wir hören voneinander.",
]

PERSONS = ["Herr Kaiser", "Frau Brandt", "Lena", "Tobias", "Frau Demir", "Herr Schmitt", "Marcus", "Frau Lehmann"]
TIMES = [
    "am Montagvormittag", "im ersten Quartal", "in der nächsten Woche", "Anfang Dezember",
    "am Donnerstagnachmittag", "vor dem Wochenende", "Mitte November", "am dritten Oktober",
]
DEADLINES = ["Freitagmittag", "Monatsende", "zum dritten November", "Mitte nächster Woche", "übermorgen"]
PERCENTS = ["zwölf Prozent", "fünfzehn Prozent", "acht Prozent", "drei Prozent", "zwanzig Prozent"]
DURATIONS = ["drei Tage", "zwei Wochen", "vierzig Stunden", "sechs Personentage", "eine Woche"]


@dataclass
class Speaker:
    name: str  # Anzeigename / Stimme, z. B. "Hedda"
    voice: str  # SAPI-Stimmenname (PowerShell 7)
    channel: str  # "mic" (Ich) oder "system" (Gegenseite)
    weight: float = 1.0


@dataclass
class Scene:
    key: str
    title: str
    speakers: list[Speaker]
    nouns_de: list[tuple[str, str]]
    nouns_en: list[tuple[str, str]]
    target_sec: float
    p_overlap: float
    echo_delay_ms: int
    seed: int
    extra: dict = field(default_factory=dict)


HEDDA = lambda w=1.0: Speaker("Hedda", "Microsoft Hedda", "mic", w)  # noqa: E731
STEFAN = lambda w=1.0: Speaker("Stefan", "Microsoft Stefan", "system", w)  # noqa: E731
KATJA = lambda w=1.0: Speaker("Katja", "Microsoft Katja", "system", w)  # noqa: E731

SCENES: list[Scene] = [
    Scene(
        key="scene1_status", title="Projektstatus Rechenzentrum-Umzug (2 Sprecher)",
        speakers=[HEDDA(1.0), STEFAN(1.0)],
        nouns_de=[("Lieferplan", "m"), ("Kalkulation", "f"), ("Ausschreibung", "f"), ("Schnittstelle", "f"),
                  ("Serverumstellung", "f"), ("Budget", "n"), ("Lastenheft", "n"), ("Abnahmeprotokoll", "n"),
                  ("Migrationskonzept", "n"), ("Rechenzentrum", "n"), ("Wartungsvertrag", "m"), ("Schulungsplan", "m")],
        nouns_en=[("Rollout", "m"), ("Kickoff", "m"), ("Milestone", "m"), ("Meeting", "n"), ("Workshop", "m"),
                  ("Roadmap", "f"), ("Update", "n"), ("Deadline", "f")],
        target_sec=240, p_overlap=0.10, echo_delay_ms=50, seed=101),
    Scene(
        key="scene2_angebot", title="Angebots- und Vertragsbesprechung (3 Sprecher)",
        speakers=[HEDDA(1.2), STEFAN(1.0), KATJA(1.0)],
        nouns_de=[("Rahmenvertrag", "m"), ("Angebotskalkulation", "f"), ("Lieferantenaudit", "n"),
                  ("Leistungsbeschreibung", "f"), ("Rechtsabteilung", "f"), ("Vertragsstrafe", "f"),
                  ("Gewährleistung", "f"), ("Konformitätserklärung", "f"), ("Preisliste", "f"),
                  ("Zahlungsziel", "n"), ("Lastenheft", "n"), ("Kabelbaum", "m"), ("Steuergerät", "n")],
        nouns_en=[("Service Level Agreement", "n"), ("Proof of Concept", "m"), ("Onboarding", "n"),
                  ("Key Account", "m"), ("Pricing", "n"), ("Forecast", "m"), ("Workshop", "m")],
        target_sec=360, p_overlap=0.18, echo_delay_ms=120, seed=202),
    Scene(
        key="scene3_sprint", title="Sprint-Planung und Release-Review (3 Sprecher, viel Englisch)",
        speakers=[HEDDA(1.0), KATJA(1.2), STEFAN(1.2)],
        nouns_de=[("Fehlerbehebung", "f"), ("Testabdeckung", "f"), ("Schnittstellendokumentation", "f"),
                  ("Datenbankmigration", "f"), ("Sicherheitslücke", "f"), ("Lasttest", "m"),
                  ("Abnahmetest", "m"), ("Konfigurationsdatei", "f"), ("Speicherverbrauch", "m")],
        nouns_en=[("Pull Request", "m"), ("Deployment", "n"), ("Backlog", "n"), ("Feature Flag", "n"),
                  ("Rollback", "m"), ("Code Review", "n"), ("Release Candidate", "m"), ("Staging System", "n"),
                  ("Hotfix", "m"), ("Pipeline", "f"), ("Sprint Review", "n"), ("Kubernetes Cluster", "m")],
        target_sec=480, p_overlap=0.30, echo_delay_ms=250, seed=303),
]

_SLOT = re.compile(r"\{([A-F])\.(nom|akk|dat)\}")


def _np(entry: tuple[str, str], case: str) -> str:
    word, gender = entry
    art = _ARTICLES[gender][("nom", "akk", "dat").index(case)]
    return f"{art} {word}"


def render_template(tpl: str, rng: random.Random, scene: Scene) -> str:
    picks: dict[str, tuple[str, str]] = {}
    de = rng.sample(scene.nouns_de, 3)
    en = rng.sample(scene.nouns_en, 2)
    for letter, entry in zip("ABC", de):
        picks[letter] = entry
    for letter, entry in zip("EF", en):
        picks[letter] = entry

    def sub(m: re.Match) -> str:
        return _np(picks[m.group(1)], m.group(2))

    text = _SLOT.sub(sub, tpl)
    text = text.replace("{P}", rng.choice(PERSONS)).replace("{T}", rng.choice(TIMES))
    text = text.replace("{D}", rng.choice(DEADLINES)).replace("{PC}", rng.choice(PERCENTS))
    text = text.replace("{DU}", rng.choice(DURATIONS))
    # Satzanfang gross (Artikel am Anfang, z. B. "{A.nom} ist ...")
    return text[0].upper() + text[1:]


def plan_utterances(scene: Scene, rng: random.Random, count: int, first: bool = False) -> list[dict]:
    """Textplan: Sprecher + Text je Aeusserung (noch ohne Zeiten)."""
    out: list[dict] = []
    prev_kind = None
    prev_speaker = None
    kinds = ["Q", "A", "S", "Z"]
    pools = {"Q": QUESTIONS, "A": ANSWERS, "S": STATEMENTS, "Z": AGREEMENTS}
    for i in range(count):
        if first and i == 0:
            spk = next(s for s in scene.speakers if s.channel == "mic")
            tpl = rng.choice(OPENERS)
            kind = "O"
        else:
            # Frage -> Antwort, sonst zufaellig; Sprecherwechsel ist Regel, Monolog selten
            kind = "A" if prev_kind == "Q" else rng.choice([k for k in kinds if k != prev_kind])
            weights = [s.weight if (s is not prev_speaker or rng.random() < 0.15) else 0.0 for s in scene.speakers]
            if not any(weights):
                weights = [s.weight for s in scene.speakers]
            spk = rng.choices(scene.speakers, weights=weights)[0]
            tpl = rng.choice(pools[kind])
        out.append({"speaker": spk.name, "voice": spk.voice, "channel": spk.channel,
                    "text": render_template(tpl, rng, scene)})
        prev_kind, prev_speaker = kind, spk
    return out


def closing_utterance(scene: Scene, rng: random.Random) -> dict:
    spk = next(s for s in scene.speakers if s.channel == "mic")
    return {"speaker": spk.name, "voice": spk.voice, "channel": spk.channel,
            "text": render_template(rng.choice(CLOSERS), rng, scene)}


def schedule(durs: list[float], channels: list[str], rng: random.Random, p_overlap: float,
             lead_in: float = 0.8) -> list[float]:
    """Startzeiten (s). Ueberlappung nur zwischen verschiedenen Kanaelen (Doppelsprechen
    Ich/Gegenseite); innerhalb eines Kanals nie zwei Aeusserungen gleichzeitig."""
    starts: list[float] = []
    last_end = {"mic": 0.0, "system": 0.0}
    cursor = lead_in
    for i, (dur, ch) in enumerate(zip(durs, channels)):
        if i == 0:
            start = lead_in
        else:
            prev_dur, prev_ch = durs[i - 1], channels[i - 1]
            prev_end = starts[-1] + prev_dur
            if ch != prev_ch and rng.random() < p_overlap:
                overlap = min(rng.uniform(0.5, 1.8), 0.4 * prev_dur, 0.6 * dur)
                start = prev_end - overlap
            else:
                start = prev_end + rng.uniform(0.25, 1.1)
            start = max(start, last_end[ch] + 0.15)
        starts.append(start)
        last_end[ch] = start + dur
        cursor = max(cursor, start + dur)
    return starts


def estimate_seconds(text: str) -> float:
    return len(text.split()) / 2.3 + 0.5
