import SwiftUI

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
