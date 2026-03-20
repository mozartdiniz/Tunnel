import Foundation
import Darwin

// MARK: - Peer model

struct Peer: Identifiable, Equatable {
    let id: String     // fingerprint — stable identifier
    let name: String   // alias from DeviceInfo
    let host: String   // IPv4 address string
    let port: UInt16
}

// MARK: - Discovery

/// Advertises this device via UDP multicast (LocalSend v2) and listens for peer announcements.
final class Discovery {

    private let fingerprint: String
    private var recvSocket: Int32 = -1
    private var dispatchSource: DispatchSourceRead?
    private var expiryTimer: Timer?
    private var heartbeatTimer: Timer?
    private var peerLastSeen: [String: Date] = [:]
    private let queue = DispatchQueue(label: "dev.tunnel.discovery", qos: .utility)

    var onPeerFound: ((Peer) -> Void)?
    var onPeerLost: ((String) -> Void)?

    init(fingerprint: String) {
        self.fingerprint = fingerprint
    }

    // MARK: - Advertise

    func advertise(alias: String, port: UInt16) {
        sendToMulticast(buildInfo(alias: alias, port: port, announce: true))
    }

    func unregister(alias: String, port: UInt16) {
        sendToMulticast(buildInfo(alias: alias, port: port, announce: false))
    }

    // MARK: - Browse

    func startBrowsing(alias: String, port: UInt16) {
        // nil = "I'm here, responding" — distinct from announce:false which means goodbye.
        let ownInfo = buildInfo(alias: alias, port: port, announce: nil)

        let sock = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
        guard sock >= 0 else {
            print("[Discovery] Failed to create UDP socket: \(errno)")
            return
        }
        recvSocket = sock

        var reuse: Int32 = 1
        setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, &reuse, socklen_t(MemoryLayout<Int32>.size))
        setsockopt(sock, SOL_SOCKET, SO_REUSEPORT, &reuse, socklen_t(MemoryLayout<Int32>.size))

        var addr = sockaddr_in()
        addr.sin_len    = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port   = localsendPort.bigEndian
        addr.sin_addr.s_addr = INADDR_ANY
        _ = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(sock, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }

        var mreq = ip_mreq()
        mreq.imr_multiaddr.s_addr = inet_addr(multicastAddress)
        mreq.imr_interface.s_addr = INADDR_ANY
        setsockopt(sock, IPPROTO_IP, IP_ADD_MEMBERSHIP, &mreq, socklen_t(MemoryLayout<ip_mreq>.size))

        let source = DispatchSource.makeReadSource(fileDescriptor: sock, queue: queue)
        dispatchSource = source

        source.setEventHandler { [weak self] in
            guard let self else { return }
            var buf = [UInt8](repeating: 0, count: 65536)
            var srcAddr = sockaddr_in()
            var srcLen = socklen_t(MemoryLayout<sockaddr_in>.size)
            let n = withUnsafeMutablePointer(to: &srcAddr) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { saPtr in
                    recvfrom(sock, &buf, buf.count, 0, saPtr, &srcLen)
                }
            }
            guard n > 0 else { return }

            let data = Data(buf[..<n])
            guard let info = try? JSONDecoder().decode(DeviceInfo.self, from: data) else { return }
            guard info.fingerprint != self.fingerprint else { return }
            // Ignore malformed announcements with suspiciously long aliases.
            guard info.alias.count <= 256 else { return }

            let fp = info.fingerprint
            let isGoodbye = info.announce == false

            if isGoodbye {
                if self.peerLastSeen.removeValue(forKey: fp) != nil {
                    DispatchQueue.main.async { self.onPeerLost?(fp) }
                }
                return
            }

            let wasKnown = self.peerLastSeen[fp] != nil
            self.peerLastSeen[fp] = Date()

            if !wasKnown {
                var addrStr = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
                _ = withUnsafePointer(to: srcAddr.sin_addr) {
                    $0.withMemoryRebound(to: Void.self, capacity: 1) { ptr in
                        inet_ntop(AF_INET, ptr, &addrStr, socklen_t(INET_ADDRSTRLEN))
                    }
                }
                let ip = String(cString: addrStr)
                let peer = Peer(id: fp, name: info.alias, host: ip, port: info.port)
                DispatchQueue.main.async { self.onPeerFound?(peer) }

                // Respond with our own info so the peer immediately discovers us.
                self.sendToMulticast(ownInfo)
            }
        }
        source.resume()

        // Expire peers we haven't heard from in 30 seconds, checked every 10 seconds.
        let timer = Timer(timeInterval: 10, repeats: true) { [weak self] _ in
            guard let self else { return }
            self.queue.async {
                let now = Date()
                let expired = self.peerLastSeen.filter { now.timeIntervalSince($0.value) > 30 }.map(\.key)
                for fp in expired {
                    self.peerLastSeen.removeValue(forKey: fp)
                    DispatchQueue.main.async { self.onPeerLost?(fp) }
                }
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        expiryTimer = timer

        // Heartbeat: re-announce every 10 seconds so remote peers don't expire us.
        let heartbeatInfo = buildInfo(alias: alias, port: port, announce: true)
        let heartbeat = Timer(timeInterval: 10, repeats: true) { [weak self] _ in
            self?.sendToMulticast(heartbeatInfo)
        }
        RunLoop.main.add(heartbeat, forMode: .common)
        heartbeatTimer = heartbeat
    }

    // MARK: - Stop

    func stop() {
        dispatchSource?.cancel()
        dispatchSource = nil
        expiryTimer?.invalidate()
        expiryTimer = nil
        heartbeatTimer?.invalidate()
        heartbeatTimer = nil
        if recvSocket >= 0 {
            close(recvSocket)
            recvSocket = -1
        }
    }

    // MARK: - Helpers

    private func sendToMulticast(_ info: DeviceInfo) {
        guard let payload = try? JSONEncoder().encode(info) else { return }
        queue.async {
            let sock = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
            guard sock >= 0 else { return }
            defer { close(sock) }

            var ttl: Int32 = 1
            setsockopt(sock, IPPROTO_IP, IP_MULTICAST_TTL, &ttl, socklen_t(MemoryLayout<Int32>.size))

            var dest = sockaddr_in()
            dest.sin_len    = UInt8(MemoryLayout<sockaddr_in>.size)
            dest.sin_family = sa_family_t(AF_INET)
            dest.sin_port   = localsendPort.bigEndian
            dest.sin_addr.s_addr = inet_addr(multicastAddress)

            _ = payload.withUnsafeBytes { ptr in
                withUnsafePointer(to: &dest) {
                    $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { saPtr in
                        sendto(sock, ptr.baseAddress, payload.count, 0, saPtr,
                               socklen_t(MemoryLayout<sockaddr_in>.size))
                    }
                }
            }
        }
    }

    // MARK: - Subnet scan

    /// Probe every host in the local /24-or-smaller subnets via HTTPS.
    /// Finds peers that multicast cannot reach (e.g. Ethernet ↔ Wi-Fi on different
    /// subnets) because routers forward TCP even when they block multicast.
    /// All probes run concurrently with a 500 ms timeout; the scan finishes quickly.
    func scanSubnets() async {
        let candidates = buildScanCandidates()
        guard !candidates.isEmpty else { return }
        print("[Discovery] Scanning \(candidates.count) candidate IPs")

        let sessionConfig = URLSessionConfiguration.ephemeral
        sessionConfig.timeoutIntervalForRequest = 0.5
        let delegate = AcceptAnyCertDelegate()
        let session = URLSession(configuration: sessionConfig, delegate: delegate, delegateQueue: nil)
        defer { session.invalidateAndCancel() }

        let ownFP = fingerprint

        var found: [(info: DeviceInfo, ip: String)] = []
        await withTaskGroup(of: (DeviceInfo, String)?.self) { group in
            for ip in candidates {
                group.addTask {
                    guard let url = URL(string: "https://\(ip):\(localsendPort)/api/localsend/v2/info")
                    else { return nil }
                    let request = URLRequest(url: url, timeoutInterval: 0.5)
                    guard let (data, _) = try? await session.data(for: request) else { return nil }
                    guard let info = try? JSONDecoder().decode(DeviceInfo.self, from: data),
                          info.fingerprint != ownFP,
                          info.alias.count <= 256
                    else { return nil }
                    print("[Discovery] Scan found \(info.alias) @ \(ip):\(info.port)")
                    return (info, ip)
                }
            }
            for await result in group {
                if let pair = result { found.append(pair) }
            }
        }

        for (info, ip) in found {
            let peer = Peer(id: info.fingerprint, name: info.alias, host: ip, port: info.port)
            DispatchQueue.main.async { [weak self] in self?.onPeerFound?(peer) }
        }
    }

    // MARK: - Scan helpers

    /// All host addresses in each detected local subnet (capped at /24).
    private func buildScanCandidates() -> [String] {
        let interfaces = localIPv4Interfaces()
        let ownIPs = Set(interfaces.map { $0.ip })
        var candidates = Set<String>()

        for (ip, prefixLen) in interfaces {
            let effective = max(prefixLen, 24)
            let hostBits = 32 - effective
            guard hostBits > 0 else { continue }

            let parts = ip.split(separator: ".").compactMap { UInt32($0) }
            guard parts.count == 4 else { continue }
            let ipU32 = (parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3]
            let mask: UInt32 = hostBits >= 32 ? 0 : ~UInt32(0) << hostBits
            let network = ipU32 & mask
            let hostCount = (1 << hostBits) - 2

            for i in 1...hostCount {
                let h = network | UInt32(i)
                let hostIP = "\((h >> 24) & 0xFF).\((h >> 16) & 0xFF).\((h >> 8) & 0xFF).\(h & 0xFF)"
                if !ownIPs.contains(hostIP) { candidates.insert(hostIP) }
            }
        }
        return candidates.sorted()
    }

    /// Returns (ip, prefixLen) for every active non-loopback IPv4 interface.
    private func localIPv4Interfaces() -> [(ip: String, prefixLen: Int)] {
        var result: [(ip: String, prefixLen: Int)] = []
        var ifaddr: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&ifaddr) == 0 else { return [] }
        defer { freeifaddrs(ifaddr) }

        var ptr = ifaddr
        while let iface = ptr {
            defer { ptr = iface.pointee.ifa_next }

            guard let addr = iface.pointee.ifa_addr,
                  addr.pointee.sa_family == UInt8(AF_INET),
                  (iface.pointee.ifa_flags & UInt32(IFF_LOOPBACK)) == 0
            else { continue }

            var ipBuf = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
            _ = addr.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { sin in
                withUnsafePointer(to: sin.pointee.sin_addr) { addrPtr in
                    addrPtr.withMemoryRebound(to: Void.self, capacity: 1) {
                        inet_ntop(AF_INET, $0, &ipBuf, socklen_t(INET_ADDRSTRLEN))
                    }
                }
            }
            let ipStr = String(cString: ipBuf)
            guard !ipStr.isEmpty, ipStr != "0.0.0.0" else { continue }

            var prefixLen = 0
            if let netmask = iface.pointee.ifa_netmask {
                netmask.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { sin in
                    prefixLen = Int(sin.pointee.sin_addr.s_addr.nonzeroBitCount)
                }
            }
            result.append((ip: ipStr, prefixLen: prefixLen))
        }
        return result
    }

    private func buildInfo(alias: String, port: UInt16, announce: Bool?) -> DeviceInfo {
        DeviceInfo(
            alias: alias,
            version: "2.0",
            deviceModel: "Mac",
            deviceType: "desktop",
            fingerprint: fingerprint,
            port: port,
            protocolScheme: "https",
            download: false,
            announce: announce
        )
    }
}

// MARK: - AcceptAnyCertDelegate

/// URLSession delegate that accepts any TLS certificate.
/// Used only during subnet scanning, where we probe unknown peers
/// before we know their certificate fingerprint.
private class AcceptAnyCertDelegate: NSObject, URLSessionDelegate {
    func urlSession(
        _ session: URLSession,
        didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
              let trust = challenge.protectionSpace.serverTrust
        else {
            completionHandler(.cancelAuthenticationChallenge, nil)
            return
        }
        completionHandler(.useCredential, URLCredential(trust: trust))
    }
}
