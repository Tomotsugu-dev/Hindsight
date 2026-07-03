// 极简 Vision OCR CLI:输入图片路径,stdout 输出识别文本(每行一条)。
// 供 scripts/poc/l2-ocr-accuracy-poc.py 调用。编译:
//   swiftc -O scripts/poc/ocr-cli.swift -o scripts/poc/output/ocr-cli
import Vision
import AppKit

guard CommandLine.arguments.count > 1,
      let img = NSImage(contentsOfFile: CommandLine.arguments[1]),
      let cg = img.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
    FileHandle.standardError.write("usage: ocr-cli <image>\n".data(using: .utf8)!)
    exit(1)
}
let req = VNRecognizeTextRequest()
req.recognitionLevel = .accurate
if #available(macOS 13.0, *) { req.automaticallyDetectsLanguage = true }
try VNImageRequestHandler(cgImage: cg, options: [:]).perform([req])
for r in req.results ?? [] {
    if let s = r.topCandidates(1).first?.string { print(s) }
}
