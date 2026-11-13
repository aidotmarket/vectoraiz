// Independent cmap coverage check; does not qualify typography or OCR accuracy.
import Foundation
import CoreText
import CoreGraphics

guard CommandLine.arguments.count == 3 else { exit(64) }
let directory = URL(fileURLWithPath: CommandLine.arguments[1])
let input = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[2]))
let texts = try JSONDecoder().decode([String: String].self, from: input)
var rows = [[String: Any]]()
for (language, text) in texts.sorted(by: { $0.key < $1.key }) {
    let names = language == "ja" ? ["NotoSansCJKjp-Regular.otf", "NotoSansCJKjp-Bold.otf"] :
        ["NotoSans-Regular.ttf", "NotoSans-Bold.ttf", "NotoSans-Italic.ttf", "NotoSans-BoldItalic.ttf",
         "NotoSerif-Regular.ttf", "NotoSerif-Bold.ttf", "NotoSerif-Italic.ttf", "NotoSerif-BoldItalic.ttf"]
    let scalars = Set(text.unicodeScalars.filter { !CharacterSet.whitespacesAndNewlines.contains($0) })
    for name in names {
        let data = try Data(contentsOf: directory.appendingPathComponent(name))
        guard let provider = CGDataProvider(data: data as CFData), let graphicsFont = CGFont(provider) else { exit(65) }
        let font = CTFontCreateWithGraphicsFont(graphicsFont, 24, nil, nil)
        var missing = [String]()
        for scalar in scalars.sorted(by: { $0.value < $1.value }) {
            let chars = Array(String(scalar).utf16)
            var glyphs = [CGGlyph](repeating: 0, count: chars.count)
            if !CTFontGetGlyphsForCharacters(font, chars, &glyphs, chars.count) {
                missing.append(String(format: "U+%04X", scalar.value))
            }
        }
        rows.append(["language": language, "font": name, "distinct_non_whitespace_scalars": scalars.count,
                     "missing_scalars": missing])
    }
}
let output = try JSONSerialization.data(withJSONObject: rows, options: [.prettyPrinted, .sortedKeys])
FileHandle.standardOutput.write(output)
FileHandle.standardOutput.write(Data("\n".utf8))
if rows.contains(where: { !($0["missing_scalars"] as! [String]).isEmpty }) { exit(1) }
