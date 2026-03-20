import SwiftUI
import UniformTypeIdentifiers

// MARK: - PeerRow

struct PeerRow: View {
    @EnvironmentObject var model: AppModel
    let peer: Peer

    @State private var isTargeted = false

    private var progress: Double? { model.peerProgress[peer.id] }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "desktopcomputer")
                .font(.title2)
                .foregroundStyle(.blue)
                .frame(width: 36, height: 36)

            VStack(alignment: .leading, spacing: 2) {
                Text(peer.name)
                    .font(.body)
                transferSubtitle
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

    @ViewBuilder
    private var transferSubtitle: some View {
        if let pct = progress {
            if pct >= 1.0 {
                Text("Transfer complete ✓")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                VStack(alignment: .leading, spacing: 2) {
                    HStack {
                        Text("\(Int(pct * 100))%")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                        Spacer()
                    }
                    ProgressView(value: pct)
                        .progressViewStyle(.linear)
                        .frame(maxWidth: 160)
                }
            }
        } else {
            Text("Drop a file to send")
                .font(.caption)
                .foregroundStyle(.secondary)
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
