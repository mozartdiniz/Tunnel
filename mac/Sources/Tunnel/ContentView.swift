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
            scanButton
            settingsButton
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    private var scanButton: some View {
        Button {
            model.scanNetwork()
        } label: {
            if model.isScanning {
                ProgressView()
                    .controlSize(.small)
                    .frame(width: 16, height: 16)
            } else {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
            }
        }
        .buttonStyle(.plain)
        .help("Search for devices on all network interfaces")
        .disabled(model.isScanning)
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

    @State private var spinnerRotation: Double = 0

    private var emptyState: some View {
        VStack(spacing: 16) {
            radarIcon
                .onAppear {
                    withAnimation(.linear(duration: 3).repeatForever(autoreverses: false)) {
                        spinnerRotation = 360
                    }
                }
            Text("Searching…")
                .font(.title3)
                .foregroundStyle(.secondary)
            Text("Looking for devices on your network")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var radarIcon: some View {
        ZStack {
            svgImage(named: "radar")
                .rotationEffect(.degrees(spinnerRotation))
            svgImage(named: "radar-dots")
        }
        .frame(width: 80, height: 80)
    }

    private func svgImage(named name: String) -> some View {
        Group {
            if let url = Bundle.module.url(forResource: name, withExtension: "svg"),
               let image = NSImage(contentsOf: url) {
                Image(nsImage: image)
                    .renderingMode(.template)
                    .resizable()
                    .frame(width: 80, height: 80)
                    .foregroundStyle(.secondary)
            }
        }
    }

    // MARK: Status Bar

    private var statusBar: some View {
        Text(model.statusMessage)
            .font(.caption)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
    }
}
