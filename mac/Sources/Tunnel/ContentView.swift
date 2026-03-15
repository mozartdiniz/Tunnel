import SwiftUI
import UniformTypeIdentifiers

// MARK: - ContentView

struct ContentView: View {
    @EnvironmentObject var model: AppModel
    @State private var showSettings = false

    var body: some View {
        VStack(spacing: 0) {
            headerBar
            Divider()
            peerList
            Divider()
            statusBar
        }
        .frame(minWidth: 320, minHeight: 400)
        .sheet(isPresented: $showSettings) {
            SettingsView()
                .environmentObject(model)
        }
        .sheet(item: $model.pendingRequest) { request in
            IncomingRequestSheet(request: request)
                .environmentObject(model)
        }
    }

    // MARK: Header

    private var headerBar: some View {
        HStack {
            Image(systemName: "antenna.radiowaves.left.and.right")
                .foregroundStyle(.blue)
            Text(model.deviceName)
                .font(.headline)
            Spacer()
            settingsButton
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    private var settingsButton: some View {
        Button {
            showSettings = true
        } label: {
            Image(systemName: "gear")
                .foregroundStyle(.secondary)
        }
        .buttonStyle(.plain)
        .help("Settings")
        .keyboardShortcut(",", modifiers: .command)
    }

    // MARK: Peer List

    private var peerList: some View {
        Group {
            if model.peers.isEmpty {
                emptyState
            } else {
                List(model.peers) { peer in
                    PeerRow(peer: peer)
                        .environmentObject(model)
                }
                .listStyle(.plain)
            }
        }
    }

    private var emptyState: some View {
        VStack(spacing: 12) {
            Image(systemName: "wifi.slash")
                .font(.system(size: 40))
                .foregroundStyle(.secondary)
            Text("No devices found")
                .font(.title3)
                .foregroundStyle(.secondary)
            Text("Make sure other devices running Tunnel\nare on the same network.")
                .font(.caption)
                .multilineTextAlignment(.center)
                .foregroundStyle(.tertiary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: Status Bar

    private var statusBar: some View {
        VStack(spacing: 4) {
            if let transfer = model.activeTransfer {
                HStack {
                    Text(transfer)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                    if model.transferProgress > 0 && model.transferProgress < 1 {
                        Text("\(Int(model.transferProgress * 100))%")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                }
                if model.transferProgress > 0 && model.transferProgress < 1 {
                    ProgressView(value: model.transferProgress)
                        .progressViewStyle(.linear)
                }
            } else {
                Text(model.statusMessage)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
    }


}

// MARK: - PeerRow

struct PeerRow: View {
    @EnvironmentObject var model: AppModel
    let peer: Peer

    @State private var isTargeted = false

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "desktopcomputer")
                .font(.title2)
                .foregroundStyle(.blue)
                .frame(width: 36, height: 36)

            VStack(alignment: .leading, spacing: 2) {
                Text(peer.name)
                    .font(.body)
                Text("Drop a file to send")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            Image(systemName: "arrow.up.circle")
                .foregroundStyle(isTargeted ? .blue : .secondary)
                .font(.title3)
        }
        .padding(.vertical, 6)
        .padding(.horizontal, 4)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(isTargeted ? Color.blue.opacity(0.1) : Color.clear)
        )
        .onDrop(of: [.fileURL], isTargeted: $isTargeted) { providers in
            handleDrop(providers: providers)
        }
    }

    private func handleDrop(providers: [NSItemProvider]) -> Bool {
        guard let provider = providers.first else { return false }
        provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { item, _ in
            guard let data = item as? Data,
                  let url = URL(dataRepresentation: data, relativeTo: nil)
            else { return }
            Task { @MainActor in
                model.sendFile(to: peer, fileURL: url)
            }
        }
        return true
    }
}

// MARK: - IncomingRequestSheet

struct IncomingRequestSheet: View {
    @EnvironmentObject var model: AppModel
    let request: PendingRequest

    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: "arrow.down.circle.fill")
                .font(.system(size: 48))
                .foregroundStyle(.blue)

            Text("Incoming File")
                .font(.title2.bold())

            VStack(spacing: 6) {
                Text(request.fileName)
                    .font(.headline)
                    .lineLimit(2)
                    .multilineTextAlignment(.center)
                Text("\(request.formattedSize) from \(request.senderName)")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }

            HStack(spacing: 12) {
                Button("Decline") {
                    model.denyTransfer(request.transferId)
                }
                .keyboardShortcut(.escape)
                .buttonStyle(.bordered)

                Button("Accept") {
                    model.acceptTransfer(request.transferId)
                }
                .keyboardShortcut(.return)
                .buttonStyle(.borderedProminent)
            }
        }
        .padding(32)
        .frame(width: 360)
    }
}

// MARK: - Preview

#Preview {
    ContentView()
        .environmentObject(AppModel())
}
