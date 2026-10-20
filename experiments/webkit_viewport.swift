// Experimental renderer: a single WKWebView serves repeated viewport PNG captures.
// Protocol: newline-delimited JSON on stdin/stdout. No production app changes.
import AppKit
import Foundation
import WebKit

final class ViewportRunner: NSObject, WKNavigationDelegate {
    private let htmlURL: URL
    private let readRoot: URL
    private let outputURL: URL
    private let width: CGFloat
    private let height: CGFloat
    private let webView: WKWebView
    private var documentHeight: Double = 0
    private var loaded = false

    init(htmlURL: URL, readRoot: URL, outputURL: URL, width: CGFloat, height: CGFloat) {
        self.htmlURL = htmlURL
        self.readRoot = readRoot
        self.outputURL = outputURL
        self.width = width
        self.height = height
        self.webView = WKWebView(
            frame: CGRect(x: 0, y: 0, width: width, height: height),
            configuration: WKWebViewConfiguration()
        )
        super.init()
        webView.navigationDelegate = self
    }

    func start() {
        _ = NSApplication.shared
        webView.loadFileURL(htmlURL, allowingReadAccessTo: readRoot)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        checkReady(attempt: 0)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        emit(["kind": "error", "message": error.localizedDescription])
    }

    func webView(
        _ webView: WKWebView,
        didFailProvisionalNavigation navigation: WKNavigation!,
        withError error: Error
    ) {
        emit(["kind": "error", "message": error.localizedDescription])
    }

    private func checkReady(attempt: Int) {
        let probe = """
        (() => ({
          height: Math.max(
            document.documentElement.scrollHeight,
            document.body ? document.body.scrollHeight : 0,
            document.documentElement.offsetHeight,
            window.innerHeight
          ),
          fontsReady: !document.fonts || document.fonts.status === "loaded",
          imagesReady: Array.from(document.images).every(img => img.complete)
        }))()
        """
        webView.evaluateJavaScript(probe) { result, error in
            if let error {
                self.emit(["kind": "error", "message": error.localizedDescription])
                return
            }
            let values = result as? [String: Any] ?? [:]
            let fontsReady = values["fontsReady"] as? Bool ?? true
            let imagesReady = values["imagesReady"] as? Bool ?? true
            if (!fontsReady || !imagesReady) && attempt < 40 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
                    self.checkReady(attempt: attempt + 1)
                }
                return
            }
            self.documentHeight = (values["height"] as? NSNumber)?.doubleValue ?? Double(self.height)
            self.loaded = true
            self.emit(["kind": "ready", "height": self.documentHeight])
        }
    }

    func handle(_ request: [String: Any]) {
        guard let action = request["type"] as? String else { return }
        switch action {
        case "snapshot":
            guard loaded else {
                emit(["kind": "error", "message": "document is not loaded"])
                return
            }
            let requestedY = max(0, (request["y"] as? NSNumber)?.doubleValue ?? 0)
            let maxY = max(0, documentHeight - Double(height))
            let targetY = min(requestedY, maxY)
            let started = Date()
            webView.evaluateJavaScript("window.scrollTo(0, \(targetY)); window.scrollY") { result, error in
                if let error {
                    self.emit(["kind": "error", "message": error.localizedDescription])
                    return
                }
                let actualY = (result as? NSNumber)?.doubleValue ?? -1
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.04) {
                    self.capture(started: started, requestedY: targetY, actualY: actualY)
                }
            }
        case "select":
            // An intentional DOM-selection/clipboard bridge experiment, not mouse selection.
            let script = """
            (() => {
              const node = document.querySelector("article p, p, h1, body");
              if (!node) return "";
              const range = document.createRange();
              range.selectNodeContents(node);
              const selection = window.getSelection();
              selection.removeAllRanges();
              selection.addRange(range);
              return selection.toString().slice(0, 4096);
            })()
            """
            webView.evaluateJavaScript(script) { result, error in
                if let error {
                    self.emit(["kind": "error", "message": error.localizedDescription])
                } else {
                    self.emit(["kind": "selection", "text": result as? String ?? ""])
                }
            }
        case "quit":
            exit(0)
        default:
            emit(["kind": "error", "message": "unknown request: \(action)"])
        }
    }

    private func capture(started: Date, requestedY: Double, actualY: Double) {
        let configuration = WKSnapshotConfiguration()
        configuration.rect = CGRect(x: 0, y: 0, width: width, height: height)
        configuration.afterScreenUpdates = true
        webView.takeSnapshot(with: configuration) { image, error in
            if let error {
                self.emit(["kind": "error", "message": error.localizedDescription])
                return
            }
            guard
                let image,
                let tiff = image.tiffRepresentation,
                let bitmap = NSBitmapImageRep(data: tiff),
                let png = bitmap.representation(using: .png, properties: [:])
            else {
                self.emit(["kind": "error", "message": "could not encode viewport PNG"])
                return
            }
            do {
                try png.write(to: self.outputURL, options: .atomic)
                self.emit([
                    "kind": "snapshot",
                    "requestedY": requestedY,
                    "actualY": actualY,
                    "height": self.documentHeight,
                    "elapsedMs": Date().timeIntervalSince(started) * 1000,
                    "pngBytes": png.count
                ])
            } catch {
                self.emit(["kind": "error", "message": error.localizedDescription])
            }
        }
    }

    private func emit(_ object: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: object) else { return }
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data([10]))
    }
}

let args = CommandLine.arguments
guard args.count == 6,
      let width = Double(args[4]), let height = Double(args[5]),
      width > 0, height > 0
else {
    fputs("usage: webkit_viewport.swift HTML READ_ROOT OUTPUT_PNG WIDTH HEIGHT\n", stderr)
    exit(2)
}

let runner = ViewportRunner(
    htmlURL: URL(fileURLWithPath: args[1]),
    readRoot: URL(fileURLWithPath: args[2], isDirectory: true),
    outputURL: URL(fileURLWithPath: args[3]),
    width: CGFloat(width),
    height: CGFloat(height)
)
runner.start()

DispatchQueue.global(qos: .userInitiated).async {
    while let line = readLine() {
        guard let data = line.data(using: .utf8),
              let value = try? JSONSerialization.jsonObject(with: data),
              let request = value as? [String: Any]
        else { continue }
        DispatchQueue.main.async {
            runner.handle(request)
        }
    }
}
RunLoop.main.run()
