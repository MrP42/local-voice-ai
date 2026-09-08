// Native rendering of the existing LocalVoiceAiMark 32-point geometry.
// Source: apps/local-voice/scripts/make-icons.py. Apple applies the outer mask.
import CoreGraphics
import ImageIO
import Foundation
import UniformTypeIdentifiers

let destination = CommandLine.arguments.dropFirst().first!
let size = 1024
let colors = CGColorSpace(name: CGColorSpace.sRGB)!
let context = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8,
                        bytesPerRow: size * 4, space: colors,
                        bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
context.setFillColor(CGColor(colorSpace: colors, components: [CGFloat(17)/255, CGFloat(20)/255, CGFloat(24)/255, 1])!)
context.fill(CGRect(x: 0, y: 0, width: size, height: size))
context.translateBy(x: 0, y: CGFloat(size))
context.scaleBy(x: CGFloat(size)/32, y: -CGFloat(size)/32)
context.setFillColor(CGColor(colorSpace: colors, components: [1, CGFloat(221)/255, 0, 1])!)
for (x, height) in [(7.0, 5.0), (11.5, 11.0), (16.0, 16.0), (20.5, 11.0), (25.0, 5.0)] {
    context.addPath(CGPath(roundedRect: CGRect(x: x - 1.2, y: 17 - height/2, width: 2.4, height: height),
                           cornerWidth: 1.2, cornerHeight: 1.2, transform: nil))
    context.fillPath()
}
context.fillEllipse(in: CGRect(x: 23.3, y: 6.5, width: 3.4, height: 3.4))
let image = context.makeImage()!
let output = CGImageDestinationCreateWithURL(URL(fileURLWithPath: destination) as CFURL, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(output, image, nil)
precondition(CGImageDestinationFinalize(output), "Could not write app icon")
