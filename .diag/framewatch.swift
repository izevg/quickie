// EXPERIMENT (never merged): what reaches the screen, frame by frame.
//
// Captures Quickie's largest window with ScreenCaptureKit and prints JSON lines:
//   {"k":"frame","rx":<unix ms>,"disp":<unix ms the frame was displayed>,"dirty":[[x,y,w,h],...]}
// for every complete (changed) frame, and every 250 ms (and on change)
//   {"k":"state",...} with the frontmost app, the window's frame, the screen, its refresh rates,
//   and Low Power Mode.
// Build: swiftc -O framewatch.swift -o framewatch. Needs Screen Recording for the terminal.
import AppKit
import CoreMedia
import Foundation
import ScreenCaptureKit

setvbuf(stdout, nil, _IOLBF, 0)

var timebase = mach_timebase_info()
mach_timebase_info(&timebase)
func machMs(_ t: UInt64) -> Double { Double(t) * Double(timebase.numer) / Double(timebase.denom) / 1e6 }
func nowMs() -> Double { Date().timeIntervalSince1970 * 1000 }
let machToUnix = nowMs() - machMs(mach_absolute_time())
func r2(_ x: Double) -> Double { (x * 100).rounded() / 100 }

func emit(_ record: [String: Any]) {
    if let data = try? JSONSerialization.data(withJSONObject: record),
       let line = String(data: data, encoding: .utf8) {
        print(line)
    }
}

final class Output: NSObject, SCStreamOutput, SCStreamDelegate {
    func stream(_ stream: SCStream, didOutputSampleBuffer buffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen,
              let all = CMSampleBufferGetSampleAttachmentsArray(buffer, createIfNecessary: false)
                as? [[SCStreamFrameInfo: Any]],
              let info = all.first,
              let raw = info[.status] as? Int,
              SCFrameStatus(rawValue: raw) == .complete
        else { return }
        let display = (info[.displayTime] as? UInt64).map { r2(machMs($0) + machToUnix) }
        var dirty: [[Int]] = []
        if let rects = info[.dirtyRects] as? [NSDictionary] {
            for dict in rects {
                if let rect = CGRect(dictionaryRepresentation: dict as CFDictionary) {
                    dirty.append([Int(rect.minX), Int(rect.minY), Int(rect.width), Int(rect.height)])
                }
            }
        }
        emit(["k": "frame", "rx": r2(nowMs()), "disp": display ?? NSNull(), "dirty": dirty])
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        emit(["k": "error", "rx": r2(nowMs()), "e": "\(error)"])
        exit(1)
    }
}

func state(pid: pid_t) -> [String: Any] {
    var record: [String: Any] = ["k": "state", "rx": r2(nowMs())]
    record["front"] = NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? ""
    record["lowPower"] = ProcessInfo.processInfo.isLowPowerModeEnabled
    let windows = (CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]]) ?? []
    let ours = windows
        .filter { ($0[kCGWindowOwnerPID as String] as? pid_t) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }
        .compactMap { info -> (CGRect, Bool)? in
            guard let bounds = info[kCGWindowBounds as String] as? NSDictionary,
                  let rect = CGRect(dictionaryRepresentation: bounds) else { return nil }
            return (rect, (info[kCGWindowIsOnscreen as String] as? Bool) ?? false)
        }
        .max { $0.0.width * $0.0.height < $1.0.width * $1.0.height }
    if let (rect, onscreen) = ours {
        record["win"] = [Int(rect.minX), Int(rect.minY), Int(rect.width), Int(rect.height)]
        record["onscreen"] = onscreen
    }
    if let screen = NSScreen.main {
        record["screen"] = [Int(screen.frame.width), Int(screen.frame.height)]
        record["maxFps"] = screen.maximumFramesPerSecond
        record["minInterval"] = r2(screen.minimumRefreshInterval * 1000)
        record["maxInterval"] = r2(screen.maximumRefreshInterval * 1000)
        record["granularity"] = r2(screen.displayUpdateGranularity * 1000)
        if let id = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID,
           let mode = CGDisplayCopyDisplayMode(id) {
            record["modeHz"] = mode.refreshRate
        }
    }
    return record
}

let output = Output()
var stream: SCStream?

Task {
    do {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        let candidates = content.windows.filter {
            ($0.owningApplication?.bundleIdentifier.hasPrefix("dev.quickie") ?? false) && $0.windowLayer == 0
        }
        guard let window = candidates.max(by: { $0.frame.width * $0.frame.height < $1.frame.width * $1.frame.height }),
              let pid = window.owningApplication?.processID
        else {
            emit(["k": "error", "rx": r2(nowMs()), "e": "no Quickie window on screen"])
            exit(2)
        }
        let config = SCStreamConfiguration()
        config.width = Int(window.frame.width)
        config.height = Int(window.frame.height)
        config.minimumFrameInterval = CMTime(value: 1, timescale: 240)
        config.queueDepth = 8
        config.showsCursor = false
        let filter = SCContentFilter(desktopIndependentWindow: window)
        let capture = SCStream(filter: filter, configuration: config, delegate: output)
        try capture.addStreamOutput(output, type: .screen, sampleHandlerQueue: DispatchQueue(label: "frames"))
        try await capture.startCapture()
        stream = capture
        emit(["k": "start", "rx": r2(nowMs()), "pid": pid, "window": window.windowID])
        var last = ""
        var lastTime = 0.0
        Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { _ in
            let record = state(pid: pid)
            var key = record
            key["rx"] = nil
            let text = "\(key)"
            if text != last || nowMs() - lastTime > 1000 {
                emit(record)
                last = text
                lastTime = nowMs()
            }
        }
    } catch {
        emit(["k": "error", "rx": r2(nowMs()), "e": "\(error)"])
        exit(1)
    }
}

signal(SIGINT) { _ in exit(0) }
signal(SIGTERM) { _ in exit(0) }
RunLoop.main.run()
