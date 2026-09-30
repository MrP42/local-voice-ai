//! Token-Budget der KI-Notizen (P1i, Befund B11).
//!
//! Ein lokaler Aufruf hat einen festen Kontext (`-c`, je freiem VRAM gewaehlt).
//! Prompt UND Antwort muessen hineinpassen: passt der Prompt so knapp, dass fuer
//! die Antwort nichts bleibt, bricht der Server sie ab (`finish_reason: length`)
//! und das JSON ist unbrauchbar. Vorher rechnete das Budget in Zeichen mit der
//! Kalibrierung eines einzigen Modells (Qwen3.5, 3,35 Zeichen je Token) und mit
//! einer Antwortreserve, die fuer Gemma zu klein war (B11: erster Block
//! 15 140 Token bei 16 384 Kontext, Antwort viermal abgeschnitten).
//!
//! Hier steht nur die Rechnung, rein und ohne I/O:
//! - Budget = Kontext - Antwortreserve - Rahmen des Prompts, in TOKEN;
//! - Zeichen je Token je Modellfamilie, mit Sicherheitsmarge (Rueckfall, wenn
//!   der Server das Messen ueber `/tokenize` nicht hergibt);
//! - Aufteilung in gleichmaessige Bloecke und die Teilung eines Blocks.
//!
//! Das exakte Messen (`/tokenize`) und der Lauf liegen in `enhance`.

use std::ops::Range;

/// Platz fuer die Antwort eines Aufrufs (Token): Einzeldurchlauf, map-Block und
/// Reduce. Gemessen (P1i, deterministisch, Kontext 32 768 bzw. echte
/// Block-Prompts von 9 200 Token), Gemma 4 E4B:
/// - Einzeldurchlauf: 2 500 bis 3 400 Token fuer 7 500 bis 22 000 Zeichen
///   (P1g-Fixtures), 4 700 fuer 38 000, ausnahmsweise 6 700 fuer 15 000 Zeichen;
/// - map-Block MIT Eintragsgrenze im Prompt: 2 100 bis 4 650 Token (0,25 bis 0,6
///   je Transkript-Token); OHNE sie laeuft die Antwort bis zum Kontextende
///   (7 200 Token und mehr, `finish_reason: length`) -- das war B11.
/// Qwen3.5-9B: im Einzeldurchlauf 900 bis 2 600 Token, im map-Block MIT Grenze
/// 4 100 bis 6 600 Token (45 bis 39 Eintraege), OHNE Grenze einmal 2 100 und in
/// zwei echten 35 000-Zeichen-Blocks beide Male bis zum Kontextende. Die Antwort
/// haengt bei beiden Modellen von Kleinigkeiten im Prompt ab und ist nicht
/// vorhersagbar; die Reserve deckt den ueblichen Fall.
/// Ein Block soll deshalb nie mehr als etwa doppelt so viel Transkript tragen,
/// wie Platz fuer die Antwort bleibt. Wer die Reserve trotzdem ueberschreitet,
/// wird abgeschnitten und dann halbiert (`MAX_SPLIT_DEPTH`), nicht verworfen.
/// Groesser kostet Bloecke: der Kontext ist auf 16 384 gedeckelt (`llm::context`).
pub const ANSWER_RESERVE_TOKENS: usize = 6_144;

/// Ein KI-Eintrag kostet in der JSON-Antwort rund 130 bis 150 Token (gemessen,
/// Gemma 4 E4B: 4 737 Token fuer 39 Eintraege, 6 718 fuer 48): Text, Quellen,
/// Schluessel und Einrueckung. Die Erzeugungszeit ist Token mal ~8 ms.
pub const TOKENS_PER_ENTRY: usize = 150;

/// Ein KI-Eintrag je so viele Zeichen Transkript (~3 Minuten Sprache), fuer
/// Bloecke und fuer die fertigen Notizen. Ohne Grenze im Prompt laeuft ein
/// lokales Modell (Temperatur 0) leicht in eine Endlosliste bis zum
/// Kontextende: Gemma 4 E4B im echten Block 7 200 Token und `length`, mit Grenze
/// 2 100 bis 4 650 Token und `stop`; Qwen3.5-9B ohne Grenze in zwei von drei
/// echten Bloecken abgeschnitten, mit Grenze in fuenf von fuenf Laeufen
/// fertig (dafuer mehr Eintraege: die Zahl verankert, 4 bis 6 Token-Tausend).
/// Deshalb tragen alle lokalen map-Prompts die Grenze und jeder Teil nach
/// einem Abschneiden; entfernte Anbieter behalten den Prompt wie vor P1i.
pub const CHARS_PER_ENTRY: usize = 3_500;
pub const MIN_ENTRIES: usize = 4;
pub const MAX_ENTRIES: usize = 40;

/// Mehr als so viele Zeilen (Eintraege der map-Stufe): der Reduce wird
/// uebersprungen. Seine Antwort gibt alle Zeilen noch einmal als JSON aus
/// (gemessen, Gemma 4 E4B, 60 Minuten: 33 Zeilen = 3 977 Token = 25 s; im
/// ersten Stand 51 Zeilen = 8 939 Token = 57 s und am Ende abgeschnitten) und
/// kostet damit fast so viel wie ein Viertel bis ein Drittel aller Bloecke,
/// ohne mehr zu leisten als Dubletten zu entfernen und die Abschnitte der
/// Bloecke zu vereinen. Dann gilt der deterministische Zusammenschluss
/// (`merge_deterministic`, derselbe wie bei einem gescheiterten Reduce): er
/// behaelt jeden Eintrag in Transkriptreihenfolge. Kurze Listen (Qwen3.5: rund
/// 20 Eintraege) werden wie bisher verdichtet.
pub const MAX_REDUCE_LINES: usize = 24;

/// Wie viele KI-Eintraege ein Stueck Transkript von `transcript_chars` Zeichen
/// hoechstens bekommt (Nutzernotizen zaehlen nicht mit).
pub fn entry_cap(transcript_chars: usize) -> usize {
    (transcript_chars / CHARS_PER_ENTRY).clamp(MIN_ENTRIES, MAX_ENTRIES)
}

/// Rahmen jedes Prompts ohne Transkript: System-Prompt, Kopf, Vorlage, Chat-
/// Vorlage. Gemessen 641 bis 745 (P1e), geschaetzt aufgerundet.
pub const PROMPT_OVERHEAD_TOKENS: usize = 750;

/// Sondertoken der Chat-Vorlage (Rollenmarken, Begrenzer), wenn Text einzeln
/// vermessen wird (System und Nutzertext getrennt).
pub const TEMPLATE_TOKENS: usize = 32;

/// Abschlag auf das Blocklimit, wenn die Zeichen je Token nur am ganzen
/// Transkript gemessen wurden (einzelne Stellen koennen dichter sein). Der
/// fertige Prompt wird vor dem Senden trotzdem noch einmal gemessen.
pub const BLOCK_MARGIN_PERCENT: usize = 3;

/// Wie oft ein Block hoechstens halbiert wird, bevor er als nicht
/// ausgewertet gilt: Original, Haelften, Viertel.
pub const MAX_SPLIT_DEPTH: u32 = 2;

/// Kleinster Platz fuer Transkript in einem Block (Token), auch wenn Notizen
/// und Rahmen den Kontext fast fuellen: darunter waere jeder Block eine Zeile.
pub const MIN_BLOCK_TOKENS: usize = 1_024;

/// Konservative Zeichen je Token (mal hundert) der Modellfamilie, fuer Prompts
/// mit deutschem Text. Gemessen mit `/tokenize` auf den P1e/P1i-Fixtures:
/// Qwen3.5 3,35 (3,12 bis 3,48), Gemma 4 2,9 bis 3,3. Die Werte liegen darunter,
/// damit die Schaetzung eher zu viele Token zaehlt als zu wenige. Unbekannte
/// Modelle: noch vorsichtiger.
pub fn chars_per_token_x100(model: &str) -> usize {
    let name = model.to_ascii_lowercase();
    if name.contains("gemma") {
        290
    } else if name.contains("qwen") {
        330
    } else {
        280
    }
}

/// Die Token-Rechnung eines lokalen Modells mit bekanntem Kontext.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenBudget {
    /// Kontext eines Slots (`-c` bei einem Slot).
    pub context: usize,
    pub reserve: usize,
    pub overhead: usize,
    pub cpt_x100: usize,
}

impl TokenBudget {
    pub fn for_model(model: &str, context_tokens: u32) -> Self {
        Self {
            context: context_tokens as usize,
            reserve: ANSWER_RESERVE_TOKENS,
            overhead: PROMPT_OVERHEAD_TOKENS,
            cpt_x100: chars_per_token_x100(model),
        }
    }

    /// Token, die Notizen und Transkript hoechstens haben duerfen:
    /// Kontext - Antwortreserve - Rahmen (nie negativ).
    pub fn payload_tokens(&self) -> usize {
        self.context
            .saturating_sub(self.reserve + self.overhead + TEMPLATE_TOKENS)
    }

    /// [`Self::payload_tokens`] in Zeichen (geschaetzt, konservativ).
    pub fn payload_chars(&self) -> usize {
        self.payload_tokens() * self.cpt_x100 / 100
    }

    /// Geschaetzte Token eines Texts von `chars` Zeichen (aufgerundet).
    pub fn estimate_tokens(&self, chars: usize) -> usize {
        (chars * 100).div_ceil(self.cpt_x100.max(1))
    }

    /// Passt ein Prompt von `prompt_tokens` (System + Nutzertext, ohne Chat-
    /// Vorlage) samt Antwortreserve in den Kontext?
    pub fn fits(&self, prompt_tokens: usize) -> bool {
        prompt_tokens + TEMPLATE_TOKENS + self.reserve <= self.context
    }

    /// Platz fuer Transkript (Token) in einem map-Block, dessen Prompt ohne
    /// Transkript `fixed_tokens` misst. Nie unter [`MIN_BLOCK_TOKENS`].
    pub fn block_room_tokens(&self, fixed_tokens: usize) -> usize {
        self.context
            .saturating_sub(self.reserve + TEMPLATE_TOKENS + fixed_tokens)
            .max(MIN_BLOCK_TOKENS)
    }
}

/// Zeichen je Block aus einer Messung: `transcript_tokens` Token fuer
/// `transcript_chars` Zeichen (`/tokenize` ueber das ganze Transkript),
/// `room_tokens` Platz je Block, minus Sicherheitsmarge.
pub fn block_chars_from_measure(
    room_tokens: usize,
    transcript_chars: usize,
    transcript_tokens: usize,
) -> usize {
    if transcript_tokens == 0 {
        return transcript_chars.max(1);
    }
    let chars = room_tokens as u128 * transcript_chars as u128 / transcript_tokens as u128;
    (chars.saturating_mul((100 - BLOCK_MARGIN_PERCENT) as u128) / 100).min(usize::MAX as u128) as usize
}

/// Zeichenlimit fuer das Packen ganzer Zeilen, so dass die Bloecke
/// gleichmaessig werden statt "voll, voll, Rest": bei `n` noetigen Bloecken
/// ungefaehr `total / n` (plus eine lange Zeile Luft fuer die Zeilengrenze),
/// nie ueber `max_block_chars`. Ein einziger Block braucht kein Teilen.
///
/// Gleichmaessige Bloecke halten auch die Antwort je Block klein: die Antwort
/// waechst mit dem Blockinhalt, ein voller erster Block waere der, der am Ende
/// der Reserve abgeschnitten wird.
pub fn balanced_block_limit(total_chars: usize, max_block_chars: usize, longest_line: usize) -> usize {
    let max_block_chars = max_block_chars.max(1);
    let blocks = total_chars.div_ceil(max_block_chars).max(1);
    if blocks == 1 {
        return max_block_chars;
    }
    (total_chars.div_ceil(blocks) + longest_line).min(max_block_chars)
}

/// Wo ein Block aus Zeilen (Laengen in Zeichen) halbiert wird: der Index, ab
/// dem die zweite Haelfte beginnt (1..len), so dass beide Haelften moeglichst
/// gleich lang sind. `None`, wenn der Block nur eine Zeile hat: eine einzelne
/// Zeile wird nie zerschnitten (IDs bleiben ganz).
pub fn split_point(line_chars: &[usize]) -> Option<usize> {
    if line_chars.len() < 2 {
        return None;
    }
    let total: usize = line_chars.iter().sum();
    let (mut best, mut best_gap, mut left) = (1usize, usize::MAX, 0usize);
    for (index, len) in line_chars.iter().enumerate().take(line_chars.len() - 1) {
        left += len;
        let gap = (2 * left).abs_diff(total);
        if gap < best_gap {
            best = index + 1;
            best_gap = gap;
        }
    }
    Some(best)
}

/// Halbiert einen Indexbereich anhand der Zeilenlaengen (`line_chars[i]` gehoert
/// zu Index `i`). `None`, wenn der Bereich weniger als zwei Zeilen hat.
pub fn halve(range: &Range<usize>, line_chars: &[usize]) -> Option<(Range<usize>, Range<usize>)> {
    let lens = line_chars.get(range.clone())?;
    let at = split_point(lens)?;
    Some((range.start..range.start + at, range.start + at..range.end))
}

/// Bezeichnung eines Blocks im Prompt: `2` fuer den zweiten Block, `2.1` fuer
/// die erste Haelfte davon, `2.1.2` fuer ein Viertel. `path` ist der Weg der
/// Halbierungen (0 = links, 1 = rechts).
pub fn part_label(block_index: usize, path: &[u8]) -> String {
    let mut label = (block_index + 1).to_string();
    for side in path {
        label.push('.');
        label.push_str(&(u32::from(*side) + 1).to_string());
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEMMA: &str = "llm-gemma4-e4b-q4";
    const QWEN: &str = "llm-qwen3.5-9b-q4";

    #[test]
    fn the_chars_per_token_follow_the_model_family_and_stay_conservative() {
        assert_eq!(chars_per_token_x100(GEMMA), 290);
        assert_eq!(chars_per_token_x100("llm-gemma3-12b-q4"), 290);
        assert_eq!(chars_per_token_x100(QWEN), 330);
        assert_eq!(chars_per_token_x100("llm-qwen3-4b-q4"), 330);
        // Unbekannt: unter beiden bekannten Werten.
        let other = chars_per_token_x100("llm-llama3-8b");
        assert!(other < chars_per_token_x100(GEMMA), "{other}");
        // Konservativ gegenueber der Messung (Qwen 3,35; Gemma 3,0 bis 3,3).
        assert!(chars_per_token_x100(QWEN) <= 335);
        assert!(chars_per_token_x100(GEMMA) <= 300);
    }

    /// Der Fall aus B11: Kontext 16 384, Gemma. Mit den alten Werten (3,35 und
    /// 2 048 Reserve) ergab das 45 513 Zeichen = ~15 100 Gemma-Token Prompt und
    /// 1 244 Token Platz fuer die Antwort. Jetzt bleibt die Reserve frei.
    #[test]
    fn a_block_at_the_budget_leaves_the_answer_reserve_free_for_gemma() {
        let budget = TokenBudget::for_model(GEMMA, 16_384);
        let chars = budget.payload_chars();
        // Auch bei der schlechtesten gemessenen Dichte (3,0 Zeichen je Token)
        // plus Rahmen bleibt die Reserve im Kontext.
        let worst_tokens = chars * 100 / 300 + PROMPT_OVERHEAD_TOKENS + TEMPLATE_TOKENS;
        assert!(
            worst_tokens + ANSWER_RESERVE_TOKENS <= 16_384,
            "{worst_tokens} Prompt-Token + Reserve passen nicht in 16384"
        );
        // Der alte Wert haette das gesprengt.
        let old_chars = (16_384 - 2_048 - 750) * 335 / 100;
        assert_eq!(old_chars, 45_513);
        assert!(old_chars * 100 / 300 + PROMPT_OVERHEAD_TOKENS + ANSWER_RESERVE_TOKENS > 16_384);
    }

    #[test]
    fn the_budget_is_context_minus_reserve_minus_overhead_in_tokens() {
        let b = TokenBudget::for_model(QWEN, 16_384);
        assert_eq!(
            b.payload_tokens(),
            16_384 - ANSWER_RESERVE_TOKENS - PROMPT_OVERHEAD_TOKENS - TEMPLATE_TOKENS
        );
        assert_eq!(b.payload_chars(), b.payload_tokens() * 330 / 100);
        let g = TokenBudget::for_model(GEMMA, 16_384);
        assert_eq!(g.payload_tokens(), b.payload_tokens());
        // Gemma bekommt bei gleichem Kontext weniger Zeichen als Qwen.
        assert!(g.payload_chars() < b.payload_chars());
        // Mehr Kontext, mehr Platz; winziger Kontext ergibt 0, nie einen Ueberlauf.
        assert!(
            TokenBudget::for_model(GEMMA, 12_288).payload_chars()
                > TokenBudget::for_model(GEMMA, 8_192).payload_chars()
        );
        assert_eq!(TokenBudget::for_model(GEMMA, 1_000).payload_tokens(), 0);
        assert_eq!(TokenBudget::for_model(GEMMA, 1_000).payload_chars(), 0);
    }

    #[test]
    fn a_prompt_fits_only_with_the_template_tokens_and_the_reserve() {
        let b = TokenBudget::for_model(GEMMA, 16_384);
        let max = 16_384 - ANSWER_RESERVE_TOKENS - TEMPLATE_TOKENS;
        assert!(b.fits(max));
        assert!(!b.fits(max + 1));
        assert!(b.fits(0));
        // Ueberlauf der Summe gibt es nicht.
        assert!(!b.fits(usize::MAX / 2));
    }

    /// Ein map-Block darf hoechstens `entry_cap` Eintraege schreiben; die Reserve
    /// muss sie samt Nutzernotizen und JSON-Rahmen tragen, und ein Block traegt
    /// hoechstens etwa doppelt so viel Transkript, wie Antwortplatz bleibt
    /// (gemessen: die Antwort ist 0,35 bis 0,6 des Transkripts).
    #[test]
    fn the_reserve_carries_the_capped_entries_of_a_block() {
        let biggest_block_entries = entry_cap(45_000);
        assert!(
            biggest_block_entries * TOKENS_PER_ENTRY + 800 <= ANSWER_RESERVE_TOKENS,
            "{biggest_block_entries} Eintraege passen nicht in die Reserve"
        );
        let b = TokenBudget::for_model(GEMMA, 16_384);
        let room = b.block_room_tokens(PROMPT_OVERHEAD_TOKENS);
        assert!(
            ANSWER_RESERVE_TOKENS * 100 >= room * 50,
            "Reserve {ANSWER_RESERVE_TOKENS} gegen Blockplatz {room}"
        );
        // Ein Block, der den Platz ausfuellt, passt samt Reserve gerade in den Kontext.
        assert!(b.fits(room + PROMPT_OVERHEAD_TOKENS));
        assert!(!b.fits(room + PROMPT_OVERHEAD_TOKENS + 1));
    }

    /// Fuer 60 Minuten (22 000 Token) braucht es bei 16 384 Kontext drei Bloecke:
    /// mit der Antwortreserve bleiben rund 9 500 Token Transkript je Block.
    #[test]
    fn sixty_minutes_need_three_blocks_at_16k() {
        let b = TokenBudget::for_model(GEMMA, 16_384);
        let room = b.block_room_tokens(PROMPT_OVERHEAD_TOKENS);
        assert_eq!(22_000usize.div_ceil(room), 3);
        // Mit 32 768 Kontext reicht ein Block.
        let big = TokenBudget::for_model(GEMMA, 32_768);
        assert_eq!(22_000usize.div_ceil(big.block_room_tokens(PROMPT_OVERHEAD_TOKENS)), 1);
    }

    #[test]
    fn the_entry_cap_follows_the_transcript_and_stays_between_the_bounds() {
        assert_eq!(entry_cap(0), MIN_ENTRIES);
        assert_eq!(entry_cap(3_499), MIN_ENTRIES);
        assert_eq!(entry_cap(35_000), 10, "ein 35 000-Zeichen-Block: 10 Eintraege");
        // 60 Minuten (69 700 Zeichen): rund 20 Eintraege in den fertigen Notizen.
        assert_eq!(entry_cap(69_711), 19);
        assert_eq!(entry_cap(usize::MAX), MAX_ENTRIES);
        // Waechst mit dem Transkript, nie fallend.
        let mut last = 0;
        for chars in (0..200_000).step_by(1_000) {
            let cap = entry_cap(chars);
            assert!(cap >= last && (MIN_ENTRIES..=MAX_ENTRIES).contains(&cap));
            last = cap;
        }
    }

    #[test]
    fn the_estimate_rounds_up_and_matches_the_payload_chars() {
        let b = TokenBudget::for_model(GEMMA, 16_384);
        assert_eq!(b.estimate_tokens(0), 0);
        assert_eq!(b.estimate_tokens(1), 1);
        assert_eq!(b.estimate_tokens(290), 100);
        assert_eq!(b.estimate_tokens(291), 101);
        // payload_chars in Token zurueckgerechnet liegt nie ueber payload_tokens.
        assert!(b.estimate_tokens(b.payload_chars()) <= b.payload_tokens());
    }

    #[test]
    fn the_block_room_shrinks_with_the_fixed_prompt_but_keeps_a_floor() {
        let b = TokenBudget::for_model(GEMMA, 16_384);
        let empty = b.block_room_tokens(0);
        assert_eq!(empty, 16_384 - ANSWER_RESERVE_TOKENS - TEMPLATE_TOKENS);
        assert_eq!(b.block_room_tokens(750), empty - 750);
        // Notizen und Rahmen fressen fast alles: der Boden gilt.
        assert_eq!(b.block_room_tokens(50_000), MIN_BLOCK_TOKENS);
    }

    #[test]
    fn a_measured_ratio_gives_the_block_size_minus_the_margin() {
        // 10 000 Token Platz, 30 000 Zeichen = 9 000 Token gemessen: 3,33 Z./Token.
        let chars = block_chars_from_measure(10_000, 30_000, 9_000);
        let exact = 10_000 * 30_000 / 9_000;
        assert_eq!(chars, exact * (100 - BLOCK_MARGIN_PERCENT) / 100);
        assert!(chars < exact);
        // Nichts gemessen (0 Token): kein Teilen erzwingen, aber nie 0.
        assert_eq!(block_chars_from_measure(10_000, 30_000, 0), 30_000);
        assert_eq!(block_chars_from_measure(10_000, 0, 0), 1);
        // Riesige Werte laufen nicht ueber.
        assert!(block_chars_from_measure(usize::MAX / 4, usize::MAX / 4, 1) > 0);
    }

    #[test]
    fn blocks_are_balanced_instead_of_full_then_rest() {
        // 70 000 Zeichen bei hoechstens 45 000 je Block: zwei Bloecke, also
        // ~35 000 je Block (plus eine Zeile), nicht 45 000 + 25 000.
        let limit = balanced_block_limit(70_000, 45_000, 300);
        assert_eq!(limit, 35_300);
        // Drei Bloecke bei 100 000 / 45 000.
        assert_eq!(balanced_block_limit(100_000, 45_000, 300), 33_634);
        // Passt in einen Block: das volle Limit.
        assert_eq!(balanced_block_limit(20_000, 45_000, 300), 45_000);
        assert_eq!(balanced_block_limit(0, 45_000, 300), 45_000);
        // Nie ueber dem Maximum, auch mit langer Zeile.
        assert_eq!(balanced_block_limit(70_000, 35_100, 5_000), 35_100);
        // Degeneriert: nie 0.
        assert!(balanced_block_limit(10, 0, 0) >= 1);
    }

    /// Greedy-Packen mit dem gleichmaessigen Limit ergibt genau `n` Bloecke.
    #[test]
    fn packing_with_the_balanced_limit_yields_the_planned_block_count() {
        for (lines, len, max) in [(600usize, 116usize, 45_000usize), (919, 76, 30_000), (50, 900, 20_000)] {
            let total = lines * len;
            let planned = total.div_ceil(max);
            let limit = balanced_block_limit(total, max, len);
            let mut blocks = 1usize;
            let mut used = 0usize;
            for _ in 0..lines {
                if used + len > limit && used > 0 {
                    blocks += 1;
                    used = 0;
                }
                used += len;
            }
            assert_eq!(blocks, planned.max(1), "{lines} Zeilen a {len}, Maximum {max}");
        }
    }

    #[test]
    fn a_block_is_split_where_both_halves_are_about_equally_long() {
        assert_eq!(split_point(&[10, 10, 10, 10]), Some(2));
        assert_eq!(split_point(&[10, 10]), Some(1));
        // Eine sehr lange Zeile vorn: die Trennung liegt direkt dahinter.
        assert_eq!(split_point(&[100, 10, 10, 10]), Some(1));
        // Eine lange Zeile hinten.
        assert_eq!(split_point(&[10, 10, 10, 100]), Some(3));
        // Beide Haelften sind nie leer.
        for lens in [vec![1usize, 1], vec![5, 1, 1, 1, 1, 1], vec![1, 1, 1, 1, 1, 50]] {
            let at = split_point(&lens).unwrap();
            assert!(at >= 1 && at < lens.len(), "{lens:?} -> {at}");
        }
        // Eine einzelne Zeile (auch eine riesige) wird nie zerschnitten.
        assert_eq!(split_point(&[50_000]), None);
        assert_eq!(split_point(&[]), None);
    }

    #[test]
    fn halving_a_range_keeps_every_index_exactly_once_in_order() {
        let lens = vec![5usize; 11];
        let (left, right) = halve(&(2..9), &lens).unwrap();
        assert_eq!(left.start, 2);
        assert_eq!(left.end, right.start);
        assert_eq!(right.end, 9);
        assert!(!left.is_empty() && !right.is_empty());
        assert_eq!(halve(&(4..5), &lens), None, "eine Zeile");
        assert_eq!(halve(&(0..0), &lens), None, "leer");
        assert_eq!(halve(&(5..30), &lens), None, "ausserhalb der Zeilen");
    }

    /// Zwei Stufen ergeben aus einem Block hoechstens vier Teile, und jede
    /// Stufe halbiert den Inhalt: nach MAX_SPLIT_DEPTH ist ein Teil ein Viertel.
    #[test]
    fn two_levels_of_halving_leave_a_quarter_of_the_block() {
        let lens = vec![100usize; 64];
        let mut parts = vec![0..64usize];
        for _ in 0..MAX_SPLIT_DEPTH {
            parts = parts
                .into_iter()
                .flat_map(|r| {
                    let (a, b) = halve(&r, &lens).unwrap();
                    [a, b]
                })
                .collect();
        }
        assert_eq!(parts.len(), 4);
        assert!(parts.iter().all(|p| p.len() == 16));
        assert_eq!(MAX_SPLIT_DEPTH, 2, "Vorgabe: hoechstens zwei Stufen");
    }

    #[test]
    fn part_labels_show_the_path_of_the_halvings() {
        assert_eq!(part_label(0, &[]), "1");
        assert_eq!(part_label(1, &[]), "2");
        assert_eq!(part_label(1, &[0]), "2.1");
        assert_eq!(part_label(1, &[1, 0]), "2.2.1");
    }
}
