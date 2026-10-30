// Experimental renderer: a single WKWebView serves repeated viewport captures.
// Protocol: newline-delimited JSON on stdin/stdout. No production app changes.
import AppKit
import Foundation
import WebKit
import zlib

final class ViewportRunner: NSObject, WKNavigationDelegate {
    private let htmlURL: URL
    private let readRoot: URL
    private let outputURL: URL
    private var width: CGFloat
    private var height: CGFloat
    // Snapshot width in pixels; 0 keeps the backing scale.
    private var pixelWidth: CGFloat
    private let webView: WKWebView
    private var documentHeight: Double = 0
    private var loaded = false
    private let metricsStartedAt = Date()
    private var snapshotRequests = 0
    private var snapshotCompleted = 0
    private var lastSnapshotRequestedAt: Date?

    init(
        htmlURL: URL, readRoot: URL, outputURL: URL,
        width: CGFloat, height: CGFloat, pixelWidth: CGFloat
    ) {
        self.htmlURL = htmlURL
        self.readRoot = readRoot
        self.outputURL = outputURL
        self.width = width
        self.height = height
        self.pixelWidth = pixelWidth
        self.webView = WKWebView(
            frame: CGRect(x: 0, y: 0, width: width, height: height * 2),
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
        case "dimensions":
            emit(["kind": "dimensions", "height": documentHeight])
        case "metrics":
            emit([
                "kind": "metrics",
                "requests": snapshotRequests,
                "completed": snapshotCompleted,
                "uptimeSec": Date().timeIntervalSince(metricsStartedAt),
                "secondsSinceSnapshot": lastSnapshotRequestedAt.map {
                    Date().timeIntervalSince($0)
                } ?? -1
            ])
        case "resize":
            guard loaded else {
                emit(["kind": "error", "message": "document is not loaded"])
                return
            }
            let nextWidth = max(1, (request["width"] as? NSNumber)?.doubleValue ?? Double(width))
            let nextHeight = max(1, (request["height"] as? NSNumber)?.doubleValue ?? Double(height))
            width = CGFloat(nextWidth)
            height = CGFloat(nextHeight)
            pixelWidth = CGFloat(max(0, (request["pixelWidth"] as? NSNumber)?.doubleValue ?? 0))
            // Reflow the *same* loaded WKWebView. Do not compile/start a new
            // Swift process or reload the document on every terminal resize.
            webView.frame = CGRect(x: 0, y: 0, width: width, height: height * 2)
            webView.needsLayout = true
            webView.layoutSubtreeIfNeeded()
            // Let the resized viewport's DOM metrics settle before the next
            // outline query/snapshot. The snapshot itself has afterScreenUpdates.
            DispatchQueue.main.async {
                let probe = """
                (() => Math.max(
                  document.documentElement.scrollHeight,
                  document.body ? document.body.scrollHeight : 0,
                  document.documentElement.offsetHeight,
                  window.innerHeight
                ))()
                """
                self.webView.evaluateJavaScript(probe) { result, error in
                    if let error {
                        self.emit(["kind": "error", "message": error.localizedDescription])
                        return
                    }
                    self.documentHeight =
                        (result as? NSNumber)?.doubleValue ?? Double(self.height)
                    self.emit(["kind": "resized", "height": self.documentHeight])
                }
            }
        case "snapshot":
            snapshotRequests += 1
            lastSnapshotRequestedAt = Date()
            guard loaded else {
                emit(["kind": "error", "message": "document is not loaded"])
                return
            }
            let requestedY = max(0, (request["y"] as? NSNumber)?.doubleValue ?? 0)
            let maxY = max(0, documentHeight - Double(height * 2))
            let targetY = min(requestedY, maxY)
            let started = Date()
            webView.evaluateJavaScript("window.scrollTo(0, \(targetY)); window.scrollY") { result, error in
                if let error {
                    self.emit(["kind": "error", "message": error.localizedDescription])
                    return
                }
                let actualY = (result as? NSNumber)?.doubleValue ?? -1
                // afterScreenUpdates on WKSnapshotConfiguration already waits for
                // pending rendering updates. Avoid adding 40 ms to every prefetch.
                self.capture(started: started, requestedY: targetY, actualY: actualY)
            }
        case "outline":
            // Collect a bounded, scroll-positioned text fallback once at startup.
            // These offsets come from the same DOM as the WebKit snapshots,
            // avoiding proportional source-line guesses during fast scrolling.
            let script = """
            (() => {
              const root = document.querySelector('article, .markdown-body') || document.body;
              const elements = Array.from(
                root.querySelectorAll('h1,h2,h3,h4,h5,h6,p,li,pre,table,img')
              );
              const stride = Math.max(1, Math.ceil(elements.length / 12000));
              const items = [];
              for (let i = 0; i < elements.length; i += stride) {
                const el = elements[i];
                // Parent list/code/table blocks already contain child content.
                const parent = el.parentElement;
                if (parent && parent.closest('li,pre,table')) continue;
                const rect = el.getBoundingClientRect();
                if (rect.height < 1 || rect.width < 1) continue;
                const tag = el.tagName.toLowerCase();
                let text = tag === 'img'
                  ? '[image] ' + (el.getAttribute('alt') || '')
                  : (el.textContent || '');
                text = text.replace(/\\s+/g, ' ').trim().slice(0, 160);
                if (!text) continue;
                if (/^h[1-6]$/.test(tag)) {
                  text = '#'.repeat(Number(tag[1])) + ' ' + text;
                } else if (tag === 'li') {
                  text = '• ' + text;
                } else if (tag === 'pre') {
                  text = '[code] ' + text;
                } else if (tag === 'table') {
                  text = '[table] ' + text;
                }
                const style = window.getComputedStyle(el);
                const normalizedColor = value => {
                  if (value.startsWith('rgba(') &&
                      Number(value.slice(value.lastIndexOf(',') + 1).replace(')', '').trim()) === 0) {
                    return '';
                  }
                  const match = value.match(/^rgba?\\((\\d+),\\s*(\\d+),\\s*(\\d+)/);
                  return match ? '#' + [match[1], match[2], match[3]]
                    .map(n => Number(n).toString(16).padStart(2, '0')).join('') : '';
                };
                items.push({
                  y: Math.round(rect.top + window.scrollY),
                  x: Math.round(rect.left + window.scrollX),
                  width: Math.round(rect.width),
                  height: Math.round(rect.height),
                  text,
                  tag,
                  fontSize: parseFloat(style.fontSize) || 16,
                  lineHeight: parseFloat(style.lineHeight) || 24,
                  weight: parseInt(style.fontWeight, 10) || 400,
                  color: normalizedColor(style.color),
                  background: normalizedColor(style.backgroundColor)
                });
              }
              return {
                background: (() => {
                  const background = window.getComputedStyle(document.body).backgroundColor;
                  const match = background.match(/^rgba?\\((\\d+),\\s*(\\d+),\\s*(\\d+)/);
                  return match ? '#' + [match[1], match[2], match[3]]
                    .map(n => Number(n).toString(16).padStart(2, '0')).join('') : '#ffffff';
                })(),
                items
              };
            })()
            """
            webView.evaluateJavaScript(script) { result, error in
                if let error {
                    self.emit(["kind": "error", "message": error.localizedDescription])
                    return
                }
                self.emit([
                    "kind": "outline",
                    "items": (result as? [String: Any])?["items"] as? [[String: Any]] ?? [],
                    "background": (result as? [String: Any])?["background"] as? String ?? "#ffffff"
                ])
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

    // Kitty f=24,o=z payload. Snapshots are opaque, and terminal multiplexers
    // such as Herdr decode PNG to RGBA before forwarding it, so RGB sends a
    // quarter fewer bytes. Level 1 zlib is also smaller and faster than PNG
    // for these captures.
    static func zlibRGB(_ image: CGImage) -> Data? {
        let width = image.width, height = image.height
        // Raw Kitty pixels carry no color profile and are read as sRGB.
        var rgb = [UInt8](repeating: 0, count: width * height * 3)
        var rgba = [UInt8](repeating: 0, count: width * height * 4)
        let drawn = rgba.withUnsafeMutableBytes { buffer -> Bool in
            guard let context = CGContext(
                data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8,
                bytesPerRow: width * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
            ) else { return false }
            context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
            rgb.withUnsafeMutableBufferPointer { target in
                for pixel in 0..<(width * height) {
                    target[pixel * 3] = buffer[pixel * 4]
                    target[pixel * 3 + 1] = buffer[pixel * 4 + 1]
                    target[pixel * 3 + 2] = buffer[pixel * 4 + 2]
                }
            }
            return true
        }
        guard drawn else { return nil }
        var length = compressBound(uLong(rgb.count))
        var output = [UInt8](repeating: 0, count: Int(length))
        guard compress2(&output, &length, rgb, uLong(rgb.count), 1) == Z_OK else { return nil }
        return Data(output[0..<Int(length)])
    }

    private func capture(started: Date, requestedY: Double, actualY: Double) {
        let configuration = WKSnapshotConfiguration()
        configuration.rect = CGRect(x: 0, y: 0, width: width, height: height * 2)
        if pixelWidth > 0 {
            // snapshotWidth is in points; WebKit multiplies it by the backing scale.
            let backingScale = NSScreen.main?.backingScaleFactor ?? 1
            configuration.snapshotWidth = NSNumber(value: Double(pixelWidth / backingScale))
        }
        configuration.afterScreenUpdates = true
        let captureStarted = Date()
        webView.takeSnapshot(with: configuration) { image, error in
            let captureMs = Date().timeIntervalSince(captureStarted) * 1000
            let encodeStarted = Date()
            if let error {
                self.emit(["kind": "error", "message": error.localizedDescription])
                return
            }
            guard
                let image,
                let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil),
                let encoded = Self.zlibRGB(cgImage)
            else {
                self.emit(["kind": "error", "message": "could not encode viewport image"])
                return
            }
            let encodeMs = Date().timeIntervalSince(encodeStarted) * 1000
            do {
                let writeStarted = Date()
                try encoded.write(to: self.outputURL, options: .atomic)
                let writeMs = Date().timeIntervalSince(writeStarted) * 1000
                self.snapshotCompleted += 1
                self.emit([
                    "kind": "snapshot",
                    "requestedY": requestedY,
                    "actualY": actualY,
                    "height": self.documentHeight,
                    "elapsedMs": Date().timeIntervalSince(started) * 1000,
                    "captureMs": captureMs,
                    "encodeMs": encodeMs,
                    "writeMs": writeMs,
                    "imageWidth": cgImage.width,
                    "imageHeight": cgImage.height,
                    "imageBytes": encoded.count
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
guard args.count == 7,
      let width = Double(args[4]), let height = Double(args[5]),
      let pixelWidth = Double(args[6]),
      width > 0, height > 0, pixelWidth >= 0
else {
    fputs("usage: webkit_viewport.swift HTML READ_ROOT OUTPUT WIDTH HEIGHT PIXEL_WIDTH\n", stderr)
    exit(2)
}

let runner = ViewportRunner(
    htmlURL: URL(fileURLWithPath: args[1]),
    readRoot: URL(fileURLWithPath: args[2], isDirectory: true),
    outputURL: URL(fileURLWithPath: args[3]),
    width: CGFloat(width),
    height: CGFloat(height),
    pixelWidth: CGFloat(pixelWidth)
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
