// Independent ImageIO reference for the harness's pixel decoder.
import Foundation
import ImageIO
import CoreGraphics

guard CommandLine.arguments.count == 3,
      let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: CommandLine.arguments[1]) as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { exit(64) }
let width = image.width, height = image.height
var pixels = [UInt8](repeating: 0, count: width * height * 4)
let success = pixels.withUnsafeMutableBytes { buffer -> Bool in
    guard let context = CGContext(data: buffer.baseAddress, width: width, height: height,
                                  bitsPerComponent: 8, bytesPerRow: width * 4,
                                  space: CGColorSpaceCreateDeviceRGB(),
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { return false }
    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    return true
}
guard success else { exit(65) }
try Data(pixels).write(to: URL(fileURLWithPath: CommandLine.arguments[2]))
print("\(width) \(height) RGBA")
