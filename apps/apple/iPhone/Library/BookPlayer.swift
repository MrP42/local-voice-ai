import SwiftUI

// Einheits-Player: ein Bildschirm für Hören, Mitlesen und Lesen. Wechsel mit einem Tipp oben
// (Vorbild Audible „Hören / Lesen & Hören“), Quelle Hörbuch oder Vorlesestimme im selben Player.
// Plan: `apps/apple/docs/einheits-player.md`.

struct BookPlayerScreen: View {
    @ObservedObject var playback = LibraryPlayback.shared
    @ObservedObject var look = ReaderAppearance.shared
    @State private var chaptersShown = false
    @State private var infoShown = false

    private var hasText: Bool { !playback.paragraphs.isEmpty }
    private var surface: PlayerSurface { hasText ? playback.surface : .listen }

    var body: some View {
        VStack(spacing: 0) {
            PlayerHeader(title: playback.title, subtitle: subtitle, trailing: AnyView(trailing))
            if hasText {
                SurfaceSwitch().padding(.horizontal, 16).padding(.top, 4).padding(.bottom, 8)
            }
            Divider()
            Group {
                switch surface {
                case .listen: PlayerCoverArea()
                case .readListen: ReadAlongText(follow: true)
                case .read: ReadAlongText(follow: false)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(surface == .listen ? VoicePalette.background : look.background)
            controls
                .padding(.horizontal, 20).padding(.vertical, 10)
                .background(.bar)
        }
        .background(VoicePalette.background)
        .sheet(isPresented: $chaptersShown) { AudiobookChapterSheet() }
        .sheet(isPresented: $infoShown) { AudiobookInfoSheet() }
    }

    private var subtitle: String {
        let state: String? = playback.preparing ? "wird vorbereitet" : playback.playing ? nil : "pausiert"
        switch playback.source {
        case .audiobook:
            let chapters = playback.chapters
            let base = chapters.count > 1 ? "Kapitel \(playback.chapterIndex + 1) von \(chapters.count)" : "Hörbuch"
            return state.map { "\(base) · \($0)" } ?? base
        case .voice: return "Vorlesestimme \(playback.voiceName)" + (state.map { " · \($0)" } ?? "")
        case nil: return ""
        }
    }

    @ViewBuilder private var trailing: some View {
        HStack(spacing: 0) {
            if hasText {
                ReaderAppearanceButton()
                BookTalkButton(compact: true)
            }
            if playback.source == .audiobook {
                Button { infoShown = true } label: { Image(systemName: "info.circle").font(.title3).frame(width: 44, height: 44) }
                    .accessibilityLabel("Angaben zum Buch")
            }
        }
    }

    @ViewBuilder private var controls: some View {
        if surface == .read { ReadingBar() }
        else if playback.source == .audiobook { AudiobookControls(chaptersShown: $chaptersShown, compact: surface == .readListen) }
        else { VoiceControls() }
    }
}

/// „Hören · Lesen & Hören · Lesen“: ein Tipp, die Stelle bleibt.
struct SurfaceSwitch: View {
    @ObservedObject var playback = LibraryPlayback.shared
    var body: some View {
        Picker("Ansicht", selection: Binding(get: { playback.surface }, set: { next in
            withAnimation(.easeInOut(duration: 0.2)) { playback.show(next) }
        })) {
            ForEach(PlayerSurface.allCases, id: \.self) { Text($0.title).tag($0) }
        }
        .pickerStyle(.segmented)
        .accessibilityIdentifier("playerSurface")
        .accessibilityHint("Wechselt zwischen Cover, Mitlesen beim Hören und reinem Lesen. Die Stelle im Buch bleibt.")
    }
}

/// Text mit Markierung. Mitlesen: Satz hell, Wort kräftig, scrollt mit. Lesen: ruhiger Text, Antippen setzt die Stelle.
struct ReadAlongText: View {
    let follow: Bool
    @ObservedObject var playback = LibraryPlayback.shared
    @ObservedObject var look = ReaderAppearance.shared

    var body: some View {
        let mark = playback.readingMark
        // Im Hörbuch ist paragraphIndex der (alte) Cursor der Stimme: ohne Markierung nichts hervorheben.
        let current = mark?.paragraph ?? (playback.source == .voice ? playback.paragraphIndex : -1)
        VStack(spacing: 0) {
            FollowText(paragraphs: playback.paragraphs, current: current, follow: follow) { i, p in
                let here = i == current ? mark : nil
                Text(styled(p, mark: here))
                    .font(look.font(heading: p.heading))
                    .foregroundStyle(follow && i != current ? look.muted : look.text)
                    .lineSpacing(look.lineSpacing).fixedSize(horizontal: false, vertical: true)
                    .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                    .background(paragraphTint(i: i, current: current, mark: mark), in: RoundedRectangle(cornerRadius: 10))
                    .contentShape(Rectangle())
                    .onTapGesture { playback.jump(toParagraph: i) }
                    .accessibilityAddTraits(i == current ? [.isButton, .isSelected] : .isButton)
                    .accessibilityHint(follow ? "Springt zu diesem Absatz" : "Setzt die Stelle auf diesen Absatz")
            }
            Text(hint(mark)).font(.caption).foregroundStyle(look.muted).padding(.vertical, 4)
        }
    }

    /// Satz und Wort nur als Hintergrund: Schriftschnitt bleibt, der Umbruch springt nicht.
    private func styled(_ p: TextParagraph, mark: ReadingMark?) -> AttributedString {
        var s = AttributedString(p.text)
        guard follow, let mark, mark.precise else { return s }
        if let r = mark.sentence, let range = Range(r, in: p.text), let a = AttributedString.Index(range.lowerBound, within: s),
           let b = AttributedString.Index(range.upperBound, within: s) {
            s[a..<b].backgroundColor = look.sentenceBackground
        }
        if let r = mark.word, let range = Range(r, in: p.text), let a = AttributedString.Index(range.lowerBound, within: s),
           let b = AttributedString.Index(range.upperBound, within: s) {
            s[a..<b].backgroundColor = look.wordBackground
            s[a..<b].foregroundColor = look.wordText
        }
        return s
    }

    private func paragraphTint(i: Int, current: Int, mark: ReadingMark?) -> Color {
        guard i == current else { return .clear }
        if !follow { return look.sentenceBackground.opacity(0.35) }  // Leseposition, dezent
        return mark?.precise == false ? look.sentenceBackground : .clear
    }

    private func hint(_ mark: ReadingMark?) -> String {
        if !follow { return "Absatz antippen setzt die Stelle, ▶ hört ab dort weiter" }
        if mark?.precise == false { return "Ohne Zeitachse vom PC: Absatz nur geschätzt" }
        return playback.source == .audiobook ? "Absatz antippen springt im Hörbuch dorthin" : "Absatz antippen liest ab dort vor"
    }
}

/// Leiste der Ansicht „Lesen“: Fortschritt und ein Knopf, der an der Lesestelle weiterhört.
struct ReadingBar: View {
    @ObservedObject var playback = LibraryPlayback.shared
    var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(progressLine).font(.subheadline.weight(.semibold)).lineLimit(1)
                Text(playback.source == .audiobook ? "Hörbuch pausiert" : "Vorlesen pausiert")
                    .font(.caption).foregroundStyle(VoicePalette.secondaryText)
            }
            Spacer(minLength: 0)
            Button {
                withAnimation(.easeInOut(duration: 0.2)) { playback.show(.readListen) }
                if !playback.busy { playback.play() }
            } label: {
                Label("Ab hier hören", systemImage: "play.fill").font(.headline).padding(.horizontal, 16).frame(minHeight: 44)
                    .foregroundStyle(VoicePalette.brand).background(VoicePalette.ink, in: Capsule())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("listenFromHere")
        }
    }

    private var progressLine: String {
        if playback.source == .audiobook {
            let total = playback.totalMs
            let fraction = total > 0 ? min(1, playback.positionMs / total) : 0
            return AudiobookFormat.pageLine(playback.book, playback: playback) ?? "Buch \(LibraryFormat.percent(fraction))"
        }
        let rec = playback.book.map { PhoneLibrary.shared.record(PhoneLibrary.shared.textKey($0)) } ?? nil
        return "\(LibraryFormat.percent(LibraryProgress.fraction(rec))) gelesen"
    }
}

/// Bedienleiste für die Vorlesestimme: Fortschritt, Satzschritte, Tempo, Stimme, Quelle, Schlaftimer.
struct VoiceControls: View {
    @ObservedObject var playback = LibraryPlayback.shared
    @State private var voices = false

    var body: some View {
        VStack(spacing: 8) {
            HStack {
                let rec = playback.book.map { PhoneLibrary.shared.record(PhoneLibrary.shared.textKey($0)) } ?? nil
                Text("Text: \(LibraryFormat.percent(LibraryProgress.fraction(rec))) gelesen")
                Spacer()
                Text("Stimme: \(playback.voiceName)").lineLimit(1)
            }.font(.caption).foregroundStyle(VoicePalette.secondaryText)
            HStack {
                ControlButton(title: "Absatz zurück", systemImage: "backward.end.fill") { playback.step(paragraphs: -1) }
                Spacer(); ControlButton(title: "Satz zurück", systemImage: "backward.fill") { playback.step(sentences: -1) }
                Spacer(); PlayButton()
                Spacer(); ControlButton(title: "Satz vor", systemImage: "forward.fill") { playback.step(sentences: 1) }
                Spacer(); ControlButton(title: "Absatz vor", systemImage: "forward.end.fill") { playback.step(paragraphs: 1) }
            }
            HStack {
                RateMenu()
                Spacer(); ControlButton(title: "Stimme", systemImage: "person.wave.2") { voices = true }
                if SourceMenu.offered { Spacer(); SourceMenu() }
                Spacer(); SleepMenu()
            }
        }
        .sheet(isPresented: $voices) { VoiceList() }
    }
}

/// Quelle wählen: Hörbuch (Sprecher) oder Vorlesestimme, an derselben Textstelle.
struct SourceMenu: View {
    @ObservedObject var playback = LibraryPlayback.shared

    /// Nur wenn das Buch Text hat und ein Hörbuch existiert (auf dem iPhone oder im Konto).
    @MainActor static var offered: Bool {
        let playback = LibraryPlayback.shared, library = PhoneLibrary.shared
        guard let book = playback.book else { return false }
        return library.hasText(book) && (library.hasAudio(book) || library.audioInAccount(book))
    }

    var body: some View {
        let library = PhoneLibrary.shared
        let audioLocal = playback.book.map { library.hasAudio($0) } ?? false
        Menu {
            Button { playback.switchSource(to: .audiobook) } label: {
                if playback.source == .audiobook { Label("Hörbuch", systemImage: "checkmark") }
                else { Text(audioLocal ? "Hörbuch" : "Hörbuch (erst im Buch laden)") }
            }.disabled(!audioLocal)
            Button { playback.switchSource(to: .voice) } label: {
                if playback.source == .voice { Label("Vorlesestimme", systemImage: "checkmark") } else { Text("Vorlesestimme") }
            }
        } label: {
            VStack(spacing: 2) {
                Image(systemName: playback.source == .audiobook ? "headphones" : "waveform").font(.title2)
                Text("Quelle").font(.caption2)
            }.frame(minWidth: 56, minHeight: 44)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Quelle")
        .accessibilityValue(playback.source?.title ?? "")
        .accessibilityHint("Hörbuch oder Vorlesestimme, die Stelle im Buch bleibt")
    }
}
