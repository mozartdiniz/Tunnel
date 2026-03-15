import Foundation
import Network

// MARK: - Peer model

struct Peer: Identifiable, Equatable {
    let id: String          // Bonjour fullname, stable identifier
    let name: String        // display_name from TXT record
    let endpoint: NWEndpoint
}

// MARK: - Discovery

/// Advertises this device via Bonjour and browses for other Tunnel peers.
final class Discovery {

    private var listener: NWListener?
    private var browser: NWBrowser?

    var onPeerFound: ((Peer) -> Void)?
    var onPeerLost: ((String) -> Void)?
    var onIncomingConnection: ((NWConnection) -> Void)?

    private let queue = DispatchQueue(label: "dev.tunnel.discovery", qos: .utility)

    // MARK: - Advertise

    /// Start listening for incoming connections and advertise via Bonjour.
    func startAdvertising(
        deviceName: String,
        tlsParameters: NWParameters,
        onConnection: @escaping (NWConnection) -> Void
    ) throws {
        self.onIncomingConnection = onConnection

        let params = tlsParameters
        let service = NWListener.Service(
            name: deviceName,
            type: "_tunnel-p2p._tcp",
            domain: nil,
            txtRecord: makeTXTRecord(deviceName: deviceName)
        )

        let l = try NWListener(service: service, using: params)
        self.listener = l

        l.newConnectionHandler = { [weak self] connection in
            self?.onIncomingConnection?(connection)
        }

        l.stateUpdateHandler = { state in
            switch state {
            case .ready:    print("[Discovery] Listener ready")
            case .failed(let err): print("[Discovery] Listener failed: \(err)")
            default: break
            }
        }

        l.start(queue: queue)
    }

    // MARK: - Browse

    func startBrowsing() {
        let params = NWParameters()
        params.includePeerToPeer = true

        let b = NWBrowser(for: .bonjour(type: "_tunnel-p2p._tcp", domain: nil), using: params)
        self.browser = b

        b.browseResultsChangedHandler = { [weak self] _, changes in
            guard let self else { return }
            for change in changes {
                switch change {
                case .added(let result):
                    self.handleAdded(result)
                case .removed(let result):
                    if case .service(let name, let type_, let domain, _) = result.endpoint {
                        let fullname = "\(name).\(type_)\(domain)"
                        self.onPeerLost?(fullname)
                    }
                case .changed(old: _, new: let result, flags: _):
                    self.handleAdded(result)
                case .identical:
                    break
                @unknown default:
                    break
                }
            }
        }

        b.stateUpdateHandler = { state in
            switch state {
            case .ready:   print("[Discovery] Browser ready")
            case .failed(let err): print("[Discovery] Browser failed: \(err)")
            default: break
            }
        }

        b.start(queue: queue)
    }

    private func handleAdded(_ result: NWBrowser.Result) {
        guard case .service(let name, let type_, let domain, _) = result.endpoint else { return }

        // Skip our own advertisement
        let fullname = "\(name).\(type_)\(domain)"

        var displayName = name
        if case .bonjour(let txtRecord) = result.metadata {
            if let value = txtRecord["display_name"] {
                displayName = value
            }
        }

        let peer = Peer(id: fullname, name: displayName, endpoint: result.endpoint)
        onPeerFound?(peer)
    }

    // MARK: - Stop

    func stop() {
        listener?.cancel()
        browser?.cancel()
        listener = nil
        browser = nil
    }

    // MARK: - Helpers

    private func makeTXTRecord(deviceName: String) -> NWTXTRecord {
        var record = NWTXTRecord()
        record["display_name"] = deviceName
        return record
    }
}
