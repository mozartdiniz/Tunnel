import Foundation
import Network
import SwiftUI
import CryptoKit

// MARK: - AppModel

@MainActor
final class AppModel: ObservableObject {

    @Published var peers: [Peer] = []
    @Published var statusMessage: String = "Starting…"
    /// Per-peer transfer progress keyed by peer fingerprint (0.0–1.0).
    @Published var peerProgress: [String: Double] = [:]
    @Published var pendingRequest: PendingRequest?
    @Published var deviceName: String = ""
    @Published var downloadDir: URL = FileManager.default
        .urls(for: .downloadsDirectory, in: .userDomainMask)[0]
    @Published var isScanning: Bool = false

    var config: Config
    var tlsManager: TLSManager?
    var pendingDecisions: [String: CheckedContinuation<Bool, Never>] = [:]
    var sessions: [String: UploadSession] = [:]
    let serverQueue = DispatchQueue(label: "dev.tunnel.server", qos: .userInitiated)

    private var discovery: Discovery?
    private var httpListener: NWListener?

    init(config: Config = .load()) {
        self.config = config
        self.deviceName = config.deviceName
        self.downloadDir = config.downloadDir
    }

    // MARK: - Startup

    func start() async {
        statusMessage = "Initializing…"
        do {
            let tls = try await TLSManager.loadOrCreate(config: config)
            self.tlsManager = tls
            let fingerprint = await tls.localFingerprint

            // Start HTTPS server (receive side).
            try await startHTTPSServer(tls: tls)

            // Start UDP multicast discovery.
            let disc = Discovery(fingerprint: fingerprint)
            disc.onPeerFound = { [weak self] peer in
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    if !self.peers.contains(where: { $0.id == peer.id }) {
                        self.peers.append(peer)
                    }
                }
            }
            disc.onPeerLost = { [weak self] fp in
                Task { @MainActor [weak self] in
                    self?.peers.removeAll { $0.id == fp }
                }
            }
            self.discovery = disc
            disc.advertise(alias: config.deviceName, port: localsendPort)
            disc.startBrowsing(alias: config.deviceName, port: localsendPort)

            statusMessage = "Ready — \(config.deviceName)"

            // Give multicast 5 seconds to find peers, then do one automatic
            // subnet scan to catch peers on different network segments.
            Task {
                try? await Task.sleep(for: .seconds(5))
                scanNetwork()
            }
        } catch {
            statusMessage = "Error: \(error.localizedDescription)"
        }
    }

    // MARK: - HTTPS server

    private func startHTTPSServer(tls: TLSManager) async throws {
        let params = await tls.listenerParameters()
        let listener = try NWListener(using: params, on: NWEndpoint.Port(integerLiteral: localsendPort))
        self.httpListener = listener

        listener.newConnectionHandler = { [weak self] conn in
            Task { [weak self] in await self?.handleHTTPConnection(conn) }
        }
        listener.stateUpdateHandler = { state in
            switch state {
            case .ready:           print("[Server] HTTPS listener ready on :\(localsendPort)")
            case .failed(let err): print("[Server] Listener failed: \(err)")
            default: break
            }
        }
        listener.start(queue: serverQueue)
    }

    // MARK: - Send

    func sendFile(to peer: Peer, fileURL: URL) {
        guard let tlsManager else { return }
        peerProgress[peer.id] = 0

        Task {
            let alias = config.deviceName
            let fp: String = await tlsManager.localFingerprint
            do {
                try await sendFiles(
                    to: peer,
                    fileURLs: [fileURL],
                    senderAlias: alias,
                    senderFingerprint: fp,
                    tlsManager: tlsManager,
                    progress: { [weak self] pct in
                        Task { @MainActor [weak self] in self?.peerProgress[peer.id] = pct }
                    }
                )
                peerProgress[peer.id] = 1.0
                try? await Task.sleep(for: .seconds(1.2))
                peerProgress.removeValue(forKey: peer.id)
            } catch TransferError.denied {
                peerProgress.removeValue(forKey: peer.id)
                statusMessage = "Transfer declined"
                try? await Task.sleep(for: .seconds(3))
                statusMessage = "Ready — \(config.deviceName)"
            } catch {
                peerProgress.removeValue(forKey: peer.id)
                statusMessage = "Send failed: \(error.localizedDescription)"
                try? await Task.sleep(for: .seconds(5))
                statusMessage = "Ready — \(config.deviceName)"
            }
        }
    }

    // MARK: - User decisions

    func acceptTransfer(_ id: String) {
        pendingDecisions.removeValue(forKey: id)?.resume(returning: true)
    }

    func denyTransfer(_ id: String) {
        pendingDecisions.removeValue(forKey: id)?.resume(returning: false)
    }

    // MARK: - Settings

    // MARK: - Subnet scan

    /// Probe all local subnets for peers that multicast cannot reach.
    /// User-triggered (search button) and also fires automatically 5 seconds
    /// after startup to catch peers on a different subnet (e.g. Ethernet ↔ Wi-Fi).
    func scanNetwork() {
        guard !isScanning, let disc = discovery else { return }
        isScanning = true
        Task {
            await disc.scanSubnets()
            isScanning = false
        }
    }

    func updateDeviceName(_ name: String) {
        config.deviceName = name
        config.save()
        deviceName = name
        discovery?.advertise(alias: name, port: localsendPort)
    }

    func updateDownloadDir(_ url: URL) {
        config.downloadDir = url
        config.save()
        downloadDir = url
    }
}

// MARK: - PendingRequest

struct PendingRequest: Identifiable {
    let id = UUID()
    let transferId: String
    let senderName: String
    let fileName: String
    let sizeBytes: UInt64

    var formattedSize: String {
        ByteCountFormatter.string(fromByteCount: Int64(sizeBytes), countStyle: .file)
    }
}
