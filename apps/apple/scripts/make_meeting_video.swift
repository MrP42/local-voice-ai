import AVFoundation
import CoreVideo
import Foundation

// Reproducible silent video + synthetic speech fixture, no microphone capture.
@main struct Fixture {
    static func main() async throws {
        guard CommandLine.arguments.count == 3 else { throw NSError(domain: "usage: fixture speech.aiff video.mp4", code: 1) }
        let speech = URL(fileURLWithPath: CommandLine.arguments[1])
        let destination = URL(fileURLWithPath: CommandLine.arguments[2])
        let silent = destination.deletingLastPathComponent().appendingPathComponent("silent-" + UUID().uuidString + ".mp4")
        defer { try? FileManager.default.removeItem(at: silent) }
        let writer = try AVAssetWriter(outputURL: silent, fileType: .mp4)
        let input = AVAssetWriterInput(mediaType: .video, outputSettings: [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: 320, AVVideoHeightKey: 180])
        let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32ARGB, kCVPixelBufferWidthKey as String: 320, kCVPixelBufferHeightKey as String: 180])
        writer.add(input)
        guard writer.startWriting() else { throw writer.error! }
        writer.startSession(atSourceTime: .zero)
        for second in 0..<45 {
            while !input.isReadyForMoreMediaData {
                guard writer.status == .writing else { throw writer.error ?? NSError(domain: "writer", code: 1) }
                try await Task.sleep(for: .milliseconds(5))
            }
            var pixel: CVPixelBuffer?
            guard CVPixelBufferCreate(kCFAllocatorDefault, 320, 180, kCVPixelFormatType_32ARGB, nil, &pixel) == kCVReturnSuccess, let pixel else { throw NSError(domain: "pixel", code: 1) }
            CVPixelBufferLockBaseAddress(pixel, [])
            memset(CVPixelBufferGetBaseAddress(pixel), 0, CVPixelBufferGetBytesPerRow(pixel) * 180)
            CVPixelBufferUnlockBaseAddress(pixel, [])
            guard adaptor.append(pixel, withPresentationTime: CMTime(value: Int64(second), timescale: 1)) else { throw writer.error ?? NSError(domain: "append", code: 1) }
        }
        writer.endSession(atSourceTime: CMTime(value: 45, timescale: 1))
        input.markAsFinished(); await writer.finishWriting()
        guard writer.status == .completed else { throw writer.error ?? NSError(domain: "finish", code: 1) }
        let composition = AVMutableComposition()
        let video = AVURLAsset(url: silent), audio = AVURLAsset(url: speech)
        let videoTrack = try await video.loadTracks(withMediaType: .video)[0]
        let audioTrack = try await audio.loadTracks(withMediaType: .audio)[0]
        let duration = try await audio.load(.duration)
        let targetVideo = composition.addMutableTrack(withMediaType: .video, preferredTrackID: kCMPersistentTrackID_Invalid)!
        let targetAudio = composition.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid)!
        try targetVideo.insertTimeRange(CMTimeRange(start: .zero, duration: CMTime(value: 45, timescale: 1)), of: videoTrack, at: .zero)
        try targetAudio.insertTimeRange(CMTimeRange(start: .zero, duration: duration), of: audioTrack, at: .zero)
        try targetAudio.insertTimeRange(CMTimeRange(start: .zero, duration: duration), of: audioTrack, at: CMTime(value: 25, timescale: 1))
        let session = AVAssetExportSession(asset: composition, presetName: AVAssetExportPreset640x480)!
        try await session.export(to: destination, as: .mp4)
        print(destination.path)
    }
}
