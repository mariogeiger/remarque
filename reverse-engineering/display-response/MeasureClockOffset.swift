import AVFoundation
import Darwin
import Foundation

func hostSeconds() -> Double {
    CMClockGetTime(CMClockGetHostTimeClock()).seconds
}

func decodeNetworkUInt64(_ bytes: [UInt8], at offset: Int) -> UInt64 {
    bytes[offset ..< offset + 8].reduce(0) { ($0 << 8) | UInt64($1) }
}

let commandLine = CommandLine.arguments
guard commandLine.count == 5,
      let port = UInt16(commandLine[2]),
      let sampleCount = Int(commandLine[3]),
      let intervalMicroseconds = useconds_t(commandLine[4]),
      sampleCount > 0 else {
    fputs("usage: measure-clock-offset ADDRESS PORT SAMPLES INTERVAL_US\n", stderr)
    exit(2)
}

let socketDescriptor = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
guard socketDescriptor >= 0 else { exit(3) }
defer { close(socketDescriptor) }

var timeout = timeval(tv_sec: 1, tv_usec: 0)
guard setsockopt(socketDescriptor, SOL_SOCKET, SO_RCVTIMEO, &timeout,
                 socklen_t(MemoryLayout.size(ofValue: timeout))) == 0 else { exit(4) }

var address = sockaddr_in()
address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
address.sin_family = sa_family_t(AF_INET)
address.sin_port = port.bigEndian
guard inet_pton(AF_INET, commandLine[1], &address.sin_addr) == 1 else { exit(5) }

print("sample,mac_send_s,device_receive_s,device_send_s,mac_receive_s,network_rtt_s,device_minus_mac_s")
for sample in 0 ..< sampleCount {
    var sequence = UInt32(sample).bigEndian
    let macSend = hostSeconds()
    let sent = withUnsafePointer(to: &address) { pointer in
        pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { socketAddress in
            sendto(socketDescriptor, &sequence, MemoryLayout.size(ofValue: sequence), 0,
                   socketAddress, socklen_t(MemoryLayout<sockaddr_in>.size))
        }
    }
    guard sent == MemoryLayout.size(ofValue: sequence) else { exit(6) }

    var response = [UInt8](repeating: 0, count: 16)
    let received = recv(socketDescriptor, &response, response.count, 0)
    let macReceive = hostSeconds()
    guard received == response.count else { exit(7) }

    let deviceReceive = Double(decodeNetworkUInt64(response, at: 0)) / 1_000_000_000
    let deviceSend = Double(decodeNetworkUInt64(response, at: 8)) / 1_000_000_000
    let networkRoundTrip = (macReceive - macSend) - (deviceSend - deviceReceive)
    let deviceMinusMac = ((deviceReceive - macSend) + (deviceSend - macReceive)) / 2
    print("\(sample),\(macSend),\(deviceReceive),\(deviceSend),\(macReceive),\(networkRoundTrip),\(deviceMinusMac)")
    if intervalMicroseconds > 0 { usleep(intervalMicroseconds) }
}
