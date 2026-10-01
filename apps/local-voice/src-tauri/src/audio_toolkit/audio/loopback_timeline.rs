/// Führt die Zeitachse des Loopback-Streams anhand der Device-Position (Frames
/// seit Stream-Start, aus dem WASAPI-Capture-Client), NIE anhand gezählter Buffer.
pub struct LoopbackTimeline {
    expected_next_frame: Option<u64>,
    /// Luecken, die ueber der Plausibilitaetsgrenze lagen (siehe `on_buffer_capped`).
    capped_gaps: u64,
    /// Frames, die diese Luecken ueber die Grenze hinaus gross waren und NICHT
    /// als Stille eingeschoben wurden.
    skipped_frames: u64,
}

#[derive(Debug)]
pub enum TimelineAction {
    /// So viele Silence-Frames VOR dem Buffer einfügen (Lücke durch Stille).
    PadSilence(u64),
    /// Buffer direkt anhängen.
    Append,
    /// Buffer verwerfen (Positionssprung rückwärts — Gerätewechsel o. ä.); Aufrufer loggt.
    Drop,
}

impl LoopbackTimeline {
    pub fn new() -> Self {
        LoopbackTimeline {
            expected_next_frame: None,
            capped_gaps: 0,
            skipped_frames: 0,
        }
    }

    /// Wie oft eine Luecke auf die Plausibilitaetsgrenze gekuerzt wurde.
    pub fn capped_gaps(&self) -> u64 {
        self.capped_gaps
    }

    /// Wie viele Frames dabei nicht eingeschoben wurden.
    pub fn skipped_frames(&self) -> u64 {
        self.skipped_frames
    }

    /// Ohne Grenze (Verhalten der ersten Fassung): jede Luecke wird gepolstert.
    pub fn on_buffer(&mut self, device_position_frames: u64, buffer_frames: u64) -> TimelineAction {
        self.on_buffer_capped(device_position_frames, buffer_frames, u64::MAX)
    }

    /// Wie [`Self::on_buffer`], mit Plausibilitaetsgrenze fuer die Stille-Polsterung
    /// (#15). Eine Luecke ueber `max_pad_frames` ist kein Schweigen des Geraets,
    /// sondern ein korrupter Positionswert (Treiberfehler, Geraetewechsel): ohne
    /// Grenze schoebe der Capture-Thread dafuer Stunden Stille durch Resampler und
    /// WAV. Hier wird auf die Grenze gekuerzt, der Rest GEZAEHLT
    /// (`capped_gaps`, `skipped_frames`) und die Zeitachse an der neuen Position
    /// fortgesetzt (der naechste zusammenhaengende Puffer haengt wieder an).
    /// Die Grenze kommt vom Aufrufer (verstrichene Wanduhrzeit, `loopback.rs`);
    /// die Zeitachse selbst bleibt allein aus der Geraeteposition abgeleitet.
    pub fn on_buffer_capped(
        &mut self,
        device_position_frames: u64,
        buffer_frames: u64,
        max_pad_frames: u64,
    ) -> TimelineAction {
        // Saettigend: ein korrupter Wert nahe u64::MAX darf nicht ueberlaufen.
        let end_of_buffer = device_position_frames.saturating_add(buffer_frames);
        match self.expected_next_frame {
            None => {
                // First buffer: define time zero
                self.expected_next_frame = Some(end_of_buffer);
                TimelineAction::Append
            }
            Some(expected) => {
                if device_position_frames == expected {
                    // Contiguous: append and update expected
                    self.expected_next_frame = Some(end_of_buffer);
                    TimelineAction::Append
                } else if device_position_frames > expected {
                    // Gap: pad silence, aber hoechstens die Plausibilitaetsgrenze
                    let gap_frames = device_position_frames - expected;
                    self.expected_next_frame = Some(end_of_buffer);
                    if gap_frames > max_pad_frames {
                        self.capped_gaps += 1;
                        self.skipped_frames = self
                            .skipped_frames
                            .saturating_add(gap_frames - max_pad_frames);
                        TimelineAction::PadSilence(max_pad_frames)
                    } else {
                        TimelineAction::PadSilence(gap_frames)
                    }
                } else {
                    // Backwards jump: drop buffer, don't update expected
                    TimelineAction::Drop
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contiguous_buffers_append_without_padding() {
        let mut t = LoopbackTimeline::new();
        assert!(matches!(t.on_buffer(0, 480), TimelineAction::Append));
        assert!(matches!(t.on_buffer(480, 480), TimelineAction::Append));
    }

    #[test]
    fn a_silence_gap_is_padded_not_compressed() {
        let mut t = LoopbackTimeline::new();
        t.on_buffer(0, 480);
        // 3 Sekunden Stille bei 48 kHz: Position springt um 144_000 Frames
        match t.on_buffer(480 + 144_000, 480) {
            TimelineAction::PadSilence(n) => assert_eq!(n, 144_000),
            other => panic!("erwartet PadSilence, bekam {other:?}"),
        }
    }

    #[test]
    fn the_first_buffer_defines_time_zero_even_at_nonzero_position() {
        // Stream lief schon, bevor wir zuhören: erste Position != 0 erzeugt KEIN Padding.
        let mut t = LoopbackTimeline::new();
        assert!(matches!(t.on_buffer(96_000, 480), TimelineAction::Append));
        assert!(matches!(t.on_buffer(96_480, 480), TimelineAction::Append));
    }

    /// #15: ein korrupter Positionssprung (Treiberfehler, Gerätewechsel) darf den
    /// Capture-Thread nicht stundenlang Stille schieben lassen. Die Grenze kommt
    /// vom Aufrufer (verstrichene Wanduhrzeit); darüber wird gekürzt, GEZÄHLT und
    /// die Zeitachse an der neuen Position fortgesetzt.
    #[test]
    fn a_corrupt_position_jump_is_capped_counted_and_resynchronised() {
        let mut t = LoopbackTimeline::new();
        t.on_buffer(0, 480);
        let cap = 3 * 48_000; // 3 s bei 48 kHz
        let jump = 480 + 5_000_000_000; // ~29 Stunden
        match t.on_buffer_capped(jump, 480, cap) {
            TimelineAction::PadSilence(n) => assert_eq!(n, cap, "höchstens die Grenze"),
            other => panic!("erwartet PadSilence, bekam {other:?}"),
        }
        assert_eq!(t.capped_gaps(), 1);
        assert_eq!(t.skipped_frames(), 5_000_000_000 - cap);
        // Danach läuft die Achse an der neuen Position weiter, ohne erneutes Polstern.
        assert!(matches!(
            t.on_buffer_capped(jump + 480, 480, cap),
            TimelineAction::Append
        ));
        assert_eq!(t.capped_gaps(), 1, "kein zweiter Treffer");
    }

    #[test]
    fn a_plausible_gap_is_padded_in_full_and_not_counted_as_capped() {
        let mut t = LoopbackTimeline::new();
        t.on_buffer(0, 480);
        let cap = 3 * 48_000;
        // Genau an der Grenze und darunter: unverändert.
        match t.on_buffer_capped(480 + cap, 480, cap) {
            TimelineAction::PadSilence(n) => assert_eq!(n, cap),
            other => panic!("{other:?}"),
        }
        match t.on_buffer_capped(480 + cap + 480 + 100, 480, cap) {
            TimelineAction::PadSilence(n) => assert_eq!(n, 100),
            other => panic!("{other:?}"),
        }
        assert_eq!((t.capped_gaps(), t.skipped_frames()), (0, 0));
    }

    #[test]
    fn a_position_near_the_top_of_u64_neither_overflows_nor_pads_endlessly() {
        let mut t = LoopbackTimeline::new();
        t.on_buffer(0, 480);
        match t.on_buffer_capped(u64::MAX - 10, 480, 48_000) {
            TimelineAction::PadSilence(n) => assert_eq!(n, 48_000),
            other => panic!("{other:?}"),
        }
        assert_eq!(t.capped_gaps(), 1);
    }

    #[test]
    fn the_cap_does_not_change_how_a_backwards_jump_or_the_first_buffer_is_handled() {
        let mut t = LoopbackTimeline::new();
        assert!(matches!(
            t.on_buffer_capped(7_000, 480, 10),
            TimelineAction::Append
        ));
        assert!(matches!(
            t.on_buffer_capped(100, 480, 10),
            TimelineAction::Drop
        ));
        assert_eq!(t.capped_gaps(), 0);
    }

    #[test]
    fn backwards_position_jumps_drop_the_buffer() {
        let mut t = LoopbackTimeline::new();
        t.on_buffer(10_000, 480);
        assert!(matches!(t.on_buffer(5_000, 480), TimelineAction::Drop));
    }
}
