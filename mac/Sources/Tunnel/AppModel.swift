import Foundation
import Network
import SwiftUI

// MARK: - AppModel

@MainActor
final class AppModel: ObservableObject {

    @Published var peers: [Peer] = []
    @Published var statusMessage: String = "Starting…"
    @Published var transferProgress: Double = 0
    @Published var activeTransfer: String?
    @Published var pendingRequest: PendingRequest?
    @Published var deviceName: String = ""
    @Published var downloadDir: URL = FileManager.default
        .urls(for: .downloadsDirectory, in: .userDomainMask)[0]

    private var config: Config
    private var tlsManager: TLSManager?
    private let discovery = Discovery()

    // Pending incoming transfers: transferId -> continuation that resolves to accept/deny
    private var pendingDecisions: [String: CheckedContinuation<Bool, Never>] = [:]

    // Active incoming connections being held for user decision (keyed by transferId)
    private var pendingConnections: [String: NWConnection] = [:]

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

            let listenerParams = await tls.listenerParameters()
            try discovery.startAdvertising(
                deviceName: config.deviceName,
                tlsParameters: listenerParams,
                onConnection: { [weak self] conn in
                    Task { await self?.handleIncoming(conn) }
                }
            )

            discovery.onPeerFound = { [weak self] peer in
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    // Filter out ourselves (same name) - Bonjour includes our own service
                    if !self.peers.contains(where: { $0.id == peer.id }) {
                        self.peers.append(peer)
                    }
                }
            }

            discovery.onPeerLost = { [weak self] peerId in
                Task { @MainActor [weak self] in
                    self?.peers.removeAll { $0.id == peerId }
                }
            }

            discovery.startBrowsing()
            statusMessage = "Ready — \(config.deviceName)"
        } catch {
            statusMessage = "Error: \(error.localizedDescription)"
        }
    }

    // MARK: - Send

    func sendFile(to peer: Peer, fileURL: URL) {
        guard let tlsManager else { return }

        activeTransfer = "Sending \(fileURL.lastPathComponent)…"
        transferProgress = 0

        Task {
            do {
                let connParams = await tlsManager.connectionParameters()
                try await Tunnel.sendFile(
                    to: peer.endpoint,
                    fileURL: fileURL,
                    senderName: config.deviceName,
                    parameters: connParams,
                    progress: { [weak self] pct in
                        Task { @MainActor [weak self] in
                            self?.transferProgress = pct
                        }
                    }
                )
                activeTransfer = "Sent \(fileURL.lastPathComponent) ✓"
                transferProgress = 1
                try? await Task.sleep(for: .seconds(3))
                activeTransfer = nil
            } catch TransferError.denied {
                activeTransfer = "Transfer declined"
                try? await Task.sleep(for: .seconds(3))
                activeTransfer = nil
            } catch {
                activeTransfer = "Send failed: \(error.localizedDescription)"
                try? await Task.sleep(for: .seconds(5))
                activeTransfer = nil
            }
        }
    }

    // MARK: - Incoming

    private func handleIncoming(_ conn: NWConnection) async {
        do {
            let request = try await readIncomingAsk(conn: conn)

            // Store connection keyed by transfer ID for later completion
            pendingConnections[request.transferId] = conn

            // Show UI prompt and wait for user decision
            let req = PendingRequest(
                transferId: request.transferId,
                senderName: request.senderName,
                fileName: request.fileName,
                sizeBytes: request.sizeBytes
            )
            self.pendingRequest = req

            let accepted = await withCheckedContinuation { cont in
                self.pendingDecisions[request.transferId] = cont
            }

            self.pendingRequest = nil

            guard let storedConn = pendingConnections.removeValue(forKey: request.transferId) else { return }

            if accepted {
                activeTransfer = "Receiving \(request.fileName)…"
                transferProgress = 0
            }

            let savedURL = try await completeReceive(
                conn: storedConn,
                request: request,
                accepted: accepted,
                downloadDir: config.downloadDir,
                progress: { [weak self] pct in
                    Task { @MainActor [weak self] in
                        self?.transferProgress = pct
                    }
                }
            )

            if let url = savedURL {
                activeTransfer = "Received \(url.lastPathComponent) ✓"
                transferProgress = 1
                NSWorkspace.shared.selectFile(url.path, inFileViewerRootedAtPath: url.deletingLastPathComponent().path)
                try? await Task.sleep(for: .seconds(3))
            }
            activeTransfer = nil

        } catch {
            print("[AppModel] Incoming transfer error: \(error)")
            activeTransfer = nil
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

    func updateDeviceName(_ name: String) {
        config.deviceName = name
        config.save()
        deviceName = name
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
