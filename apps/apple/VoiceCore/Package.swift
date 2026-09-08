// swift-tools-version: 6.0
import PackageDescription
let package = Package(name: "VoiceCore", platforms: [.macOS(.v13), .iOS(.v17), .watchOS(.v10)], products: [.library(name: "VoiceCore", targets: ["VoiceCore"])], targets: [.target(name: "VoiceCore"), .executableTarget(name: "VoiceCrashProbe", dependencies: ["VoiceCore"]), .testTarget(name: "VoiceCoreTests", dependencies: ["VoiceCore"])])
