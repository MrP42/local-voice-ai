import XCTest
@testable import VoiceCore

final class WatchDestinationTests: XCTestCase {
    func testComplicationLinksResolveOnlySupportedDestinations() {
        for destination in [WatchDestination.speak, .history, .latest] {
            XCTAssertEqual(WatchDestination(url: destination.url), destination)
        }
        for value in ["https://history", "localvoice-watch://delete", "localvoice-watch://speak?record=true", "localvoice-watch://latest/private", "localvoice-watch://user@history", "localvoice-watch://history#other"] {
            XCTAssertNil(WatchDestination(url: URL(string: value)!))
        }
    }
}
