import SwiftUI
import WidgetKit

private struct ShortcutEntry: TimelineEntry { let date: Date }
private struct ShortcutProvider: TimelineProvider {
    func placeholder(in context: Context) -> ShortcutEntry { ShortcutEntry(date: .now) }
    func getSnapshot(in context: Context, completion: @escaping (ShortcutEntry) -> Void) { completion(ShortcutEntry(date: .now)) }
    func getTimeline(in context: Context, completion: @escaping (Timeline<ShortcutEntry>) -> Void) {
        // These are navigation shortcuts, not a stale or invented live status.
        completion(Timeline(entries: [ShortcutEntry(date: .now)], policy: .never))
    }
}

private extension WatchDestination {
    var title: String {
        switch self { case .speak: "Sprechen"; case .history: "Verlauf"; case .latest: "Letzte Notiz" }
    }
    var symbol: String {
        switch self { case .speak: "waveform"; case .history: "clock.arrow.circlepath"; case .latest: "text.bubble" }
    }
    var detail: String {
        switch self {
        case .speak: "Aufnahme in der App öffnen"
        case .history: "Gespeicherte Sprachnotizen öffnen"
        case .latest: "Zuletzt gespeicherte Notiz öffnen"
        }
    }
}

private struct ShortcutView: View {
    let destination: WatchDestination
    @Environment(\.widgetFamily) private var family
    @Environment(\.widgetRenderingMode) private var renderingMode
    private var accent: Color { renderingMode == .fullColor ? Color(red: 1, green: 221.0/255, blue: 0) : .primary }
    var body: some View {
        Group {
            switch family {
            case .accessoryRectangular:
                HStack(spacing: 8) {
                    Image(systemName: destination.symbol).font(.title2).foregroundStyle(accent).widgetAccentable()
                    VStack(alignment: .leading, spacing: 2) {
                        Text(destination.title).font(.headline).lineLimit(1).minimumScaleFactor(0.8)
                        Text("Local Voice AI").font(.caption).lineLimit(1)
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            case .accessoryInline:
                Label(destination.title, systemImage: destination.symbol)
            case .accessoryCorner:
                Image(systemName: destination.symbol).font(.title2).foregroundStyle(accent)
                    .widgetAccentable().widgetLabel { Text(destination.title) }
            default:
                ZStack {
                    AccessoryWidgetBackground()
                    Image(systemName: destination.symbol).font(.title2).foregroundStyle(accent).widgetAccentable()
                }
            }
        }
        .accessibilityLabel("Local Voice AI: " + destination.detail)
        .widgetURL(destination.url)
        .containerBackground(.clear, for: .widget)
    }
}

private func shortcutConfiguration(_ destination: WatchDestination) -> some WidgetConfiguration {
        StaticConfiguration(kind: "de.localvoice.complication." + destination.rawValue, provider: ShortcutProvider()) { _ in
            ShortcutView(destination: destination)
        }
        .configurationDisplayName(destination.title)
        .description(destination.detail)
        .supportedFamilies([.accessoryCircular, .accessoryRectangular, .accessoryInline, .accessoryCorner])
}

private struct SpeakComplication: Widget {
    var body: some WidgetConfiguration { shortcutConfiguration(.speak) }
}
private struct HistoryComplication: Widget {
    var body: some WidgetConfiguration { shortcutConfiguration(.history) }
}
private struct LatestComplication: Widget {
    var body: some WidgetConfiguration { shortcutConfiguration(.latest) }
}

@main
struct VoiceComplications: WidgetBundle {
    var body: some Widget {
        SpeakComplication()
        HistoryComplication()
        LatestComplication()
    }
}
