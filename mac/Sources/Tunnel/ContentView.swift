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

    @State private var spinnerRotation: Double = 0

    private var emptyState: some View {
        VStack(spacing: 16) {
            searchSpinnerImage
                .rotationEffect(.degrees(spinnerRotation))
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

    private var searchSpinnerImage: AnyView {
        if let url = Bundle.module.url(forResource: "search-spinner", withExtension: "svg"),
           let image = NSImage(contentsOf: url) {
            return AnyView(Image(nsImage: image)
                .resizable()
                .frame(width: 80, height: 80))
        }
        return AnyView(Image(systemName: "magnifyingglass")
            .font(.system(size: 40))
            .foregroundStyle(.secondary)
            .frame(width: 80, height: 80))
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
