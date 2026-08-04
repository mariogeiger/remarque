import AppKit
import AVFoundation
import CoreMedia

private struct Region {
    let name: String
    let x: Int
    let y: Int
    let width: Int
    let height: Int
}

private enum Operation {
    case photo(destination: URL, warmup: Double)
    case sample(
        destination: URL,
        duration: Double,
        warmup: Double,
        regions: [Region],
        width: Int,
        height: Int,
        rate: Double
    )
}

private func readRegions(_ path: String?, width: Int, height: Int) throws -> [Region] {
    guard let path, path != "-" else {
        return [Region(
            name: "center",
            x: width / 2 - 64,
            y: height / 2 - 64,
            width: 128,
            height: 128
        )]
    }
    return try String(contentsOfFile: path, encoding: .utf8)
        .split(whereSeparator: \.isNewline)
        .filter { !$0.hasPrefix("#") && !$0.hasPrefix("name,") }
        .map { line in
            let fields = line.split(separator: ",", omittingEmptySubsequences: false)
            guard fields.count == 5,
                  let x = Int(fields[1]), let y = Int(fields[2]),
                  let width = Int(fields[3]), let height = Int(fields[4]),
                  x >= 0, y >= 0, width > 0, height > 0 else {
                throw NSError(domain: "RemarqueCameraBench", code: 3,
                              userInfo: [NSLocalizedDescriptionKey: "invalid region: \(line)"])
            }
            return Region(name: String(fields[0]), x: x, y: y, width: width, height: height)
        }
}

private func parseCommandLineOperation() throws -> Operation {
    let arguments = CommandLine.arguments
    guard arguments.count >= 3 else {
        throw NSError(domain: "RemarqueCameraBench", code: 4,
                      userInfo: [NSLocalizedDescriptionKey:
                        "usage: photo DEST [WARMUP] | sample DEST DURATION [WARMUP] [REGIONS]"])
    }
    switch arguments[1] {
    case "photo":
        return .photo(
            destination: URL(fileURLWithPath: arguments[2]),
            warmup: arguments.count > 3 ? Double(arguments[3]) ?? 2 : 2
        )
    case "sample":
        guard arguments.count >= 4 else {
            throw NSError(domain: "RemarqueCameraBench", code: 5)
        }
        let width = arguments.count > 6 ? Int(arguments[6]) ?? 1920 : 1920
        let height = arguments.count > 7 ? Int(arguments[7]) ?? 1080 : 1080
        let rate = arguments.count > 8 ? Double(arguments[8]) ?? 60 : 60
        return .sample(
            destination: URL(fileURLWithPath: arguments[2]),
            duration: Double(arguments[3]) ?? 5,
            warmup: arguments.count > 4 ? Double(arguments[4]) ?? 2 : 2,
            regions: try readRegions(
                arguments.count > 5 ? arguments[5] : nil,
                width: width,
                height: height
            ),
            width: width,
            height: height,
            rate: rate
        )
    default:
        throw NSError(domain: "RemarqueCameraBench", code: 6,
                      userInfo: [NSLocalizedDescriptionKey: "unknown operation \(arguments[1])"])
    }
}

private final class FrameReceiver: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    private let operation: Operation
    private let readyAt: Double
    private var firstPresentationTime: Double?
    private var frame = 0
    private var file: FileHandle?
    private var finished = false

    init(operation: Operation) throws {
        self.operation = operation
        let now = CMClockGetTime(CMClockGetHostTimeClock()).seconds
        switch operation {
        case let .photo(_, warmup):
            readyAt = now + warmup
        case let .sample(destination, _, warmup, regions, _, _, _):
            readyAt = now + warmup
            FileManager.default.createFile(atPath: destination.path, contents: nil)
            file = try FileHandle(forWritingTo: destination)
            let columns = regions.map(\.name).joined(separator: ",")
            file?.write(Data("frame,presentation_seconds,host_seconds,width,height,\(columns)\n".utf8))
        }
    }

    func captureOutput(
        _ output: AVCaptureOutput,
        didOutput sampleBuffer: CMSampleBuffer,
        from connection: AVCaptureConnection
    ) {
        guard !finished else { return }
        let hostTime = CMClockGetTime(CMClockGetHostTimeClock()).seconds
        guard hostTime >= readyAt else { return }
        switch operation {
        case let .photo(destination, _):
            writePhoto(sampleBuffer, to: destination)
            finish()
        case let .sample(_, duration, _, regions, _, _, _):
            sample(sampleBuffer, hostTime: hostTime, duration: duration, regions: regions)
        }
    }

    private func writePhoto(_ sampleBuffer: CMSampleBuffer, to destination: URL) {
        guard let image = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        let ciImage = CIImage(cvPixelBuffer: image)
        guard let cgImage = CIContext().createCGImage(ciImage, from: ciImage.extent) else { return }
        let bitmap = NSBitmapImageRep(cgImage: cgImage)
        guard let data = bitmap.representation(using: .jpeg, properties: [.compressionFactor: 0.95]) else {
            return
        }
        try? data.write(to: destination, options: .atomic)
    }

    private func sample(
        _ sampleBuffer: CMSampleBuffer,
        hostTime: Double,
        duration: Double,
        regions: [Region]
    ) {
        let presentationTime = CMSampleBufferGetPresentationTimeStamp(sampleBuffer).seconds
        if firstPresentationTime == nil { firstPresentationTime = presentationTime }
        guard let firstPresentationTime else { return }
        if presentationTime - firstPresentationTime > duration {
            finish()
            return
        }
        guard let image = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        CVPixelBufferLockBaseAddress(image, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(image, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddressOfPlane(image, 0) else { return }
        let imageWidth = CVPixelBufferGetWidthOfPlane(image, 0)
        let imageHeight = CVPixelBufferGetHeightOfPlane(image, 0)
        let rowBytes = CVPixelBufferGetBytesPerRowOfPlane(image, 0)
        let pixels = base.assumingMemoryBound(to: UInt8.self)
        let means = regions.map { region -> String in
            let x0 = min(imageWidth, region.x)
            let y0 = min(imageHeight, region.y)
            let x1 = min(imageWidth, region.x + region.width)
            let y1 = min(imageHeight, region.y + region.height)
            guard x0 < x1, y0 < y1 else { return "nan" }
            let step = max(1, min(region.width, region.height) / 32)
            var sum: UInt64 = 0
            var count: UInt64 = 0
            for y in Swift.stride(from: y0, to: y1, by: step) {
                for x in Swift.stride(from: x0, to: x1, by: step) {
                    sum += UInt64(pixels[y * rowBytes + x])
                    count += 1
                }
            }
            return String(Double(sum) / Double(count))
        }
        let values = means.joined(separator: ",")
        let line = "\(frame),\(presentationTime),\(hostTime),\(imageWidth),\(imageHeight),\(values)\n"
        file?.write(Data(line.utf8))
        frame += 1
    }

    private func finish() {
        finished = true
        file?.closeFile()
        DispatchQueue.main.async { NSApplication.shared.terminate(nil) }
    }
}

private final class ApplicationDelegate: NSObject, NSApplicationDelegate {
    private var receiver: FrameReceiver?
    private var session: AVCaptureSession?

    func applicationDidFinishLaunching(_ notification: Notification) {
        AVCaptureDevice.requestAccess(for: .video) { granted in
            guard granted else {
                self.fail("camera permission denied")
                return
            }
            self.start()
        }
    }

    private func start() {
        do {
            let operation = try parseCommandLineOperation()
            let devices = AVCaptureDevice.DiscoverySession(
                deviceTypes: [.external, .builtInWideAngleCamera],
                mediaType: .video,
                position: .unspecified
            ).devices
            guard let camera = devices.first(where: { $0.localizedName == "MX Brio" }) else {
                throw NSError(domain: "RemarqueCameraBench", code: 7,
                              userInfo: [NSLocalizedDescriptionKey: "MX Brio not found"])
            }
            let session = AVCaptureSession()
            let input = try AVCaptureDeviceInput(device: camera)
            let output = AVCaptureVideoDataOutput()
            output.alwaysDiscardsLateVideoFrames = false
            output.videoSettings = [
                kCVPixelBufferPixelFormatTypeKey as String:
                    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
            ]
            guard session.canAddInput(input), session.canAddOutput(output) else {
                throw NSError(domain: "RemarqueCameraBench", code: 8)
            }
            session.addInput(input)
            session.addOutput(output)

            let target = switch operation {
            case .photo: (width: 3840, height: 2160, rate: 30.0)
            case let .sample(_, _, _, _, width, height, rate):
                (width: width, height: height, rate: rate)
            }
            output.videoSettings = [
                kCVPixelBufferPixelFormatTypeKey as String:
                    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                kCVPixelBufferWidthKey as String: target.width,
                kCVPixelBufferHeightKey as String: target.height,
            ]
            try camera.lockForConfiguration()
            guard let format = camera.formats.first(where: {
                let dimensions = CMVideoFormatDescriptionGetDimensions($0.formatDescription)
                return dimensions.width == target.width && dimensions.height == target.height
                    && CMFormatDescriptionGetMediaSubType($0.formatDescription)
                        == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
                    && $0.videoSupportedFrameRateRanges.contains {
                        abs($0.maxFrameRate - target.rate) < 0.01
                    }
            }) else {
                camera.unlockForConfiguration()
                throw NSError(domain: "RemarqueCameraBench", code: 9)
            }
            camera.activeFormat = format
            if let rate = format.videoSupportedFrameRateRanges.first(where: {
                abs($0.maxFrameRate - target.rate) < 0.01
            }) {
                camera.activeVideoMinFrameDuration = rate.minFrameDuration
                camera.activeVideoMaxFrameDuration = rate.maxFrameDuration
                if let connection = output.connection(with: .video) {
                    if connection.isVideoMinFrameDurationSupported {
                        connection.videoMinFrameDuration = rate.minFrameDuration
                    }
                    if connection.isVideoMaxFrameDurationSupported {
                        connection.videoMaxFrameDuration = rate.maxFrameDuration
                    }
                }
            }
            camera.unlockForConfiguration()

            let receiver = try FrameReceiver(operation: operation)
            output.setSampleBufferDelegate(
                receiver,
                queue: DispatchQueue(label: "ch.mariogeiger.remarque.camera-bench")
            )
            self.receiver = receiver
            self.session = session
            session.startRunning()
            try camera.lockForConfiguration()
            if let rate = format.videoSupportedFrameRateRanges.first(where: {
                abs($0.maxFrameRate - target.rate) < 0.01
            }) {
                camera.activeVideoMinFrameDuration = rate.minFrameDuration
                camera.activeVideoMaxFrameDuration = rate.maxFrameDuration
            }
            camera.unlockForConfiguration()
        } catch {
            fail("\(error)")
        }
    }

    private func fail(_ message: String) {
        try? "\(message)\n".write(
            to: URL(fileURLWithPath: "/tmp/remarque-camera-bench.error"),
            atomically: true,
            encoding: .utf8
        )
        NSApplication.shared.terminate(nil)
    }
}

let application = NSApplication.shared
private let delegate = ApplicationDelegate()
application.delegate = delegate
application.run()
