import SwiftUI
import UniformTypeIdentifiers

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

    private var subtitleText: String {
        switch model.peerProgress[peer.id] {
        case nil:
            return "Drop a file to send"
        case .complete:
            return "Transfer complete ✓"
        case .transferring(let bytesDone, let totalBytes, let bytesPerSec, let etaSecs):
            let pct = totalBytes > 0 ? Int(Double(bytesDone) / Double(totalBytes) * 100) : 0
            var text = "\(pct)% · \(humanBytes(bytesDone)) / \(humanBytes(totalBytes))"
            if bytesPerSec > 0 { text += "  \(humanBytes(bytesPerSec))ps" }
            if let eta = etaSecs { text += "  ETA \(formatEta(eta))" }
            return text
        }
    }

    private var transferSubtitle: some View {
        Text(subtitleText)
            .font(.caption)
            .foregroundStyle(.secondary)
    }

    private func humanBytes(_ b: UInt64) -> String {
        let k: UInt64 = 1024, m = k * 1024, g = m * 1024
        if b >= g { return String(format: "%.1f GB", Double(b) / Double(g)) }
        if b >= m { return String(format: "%.1f MB", Double(b) / Double(m)) }
        if b >= k { return String(format: "%.1f KB", Double(b) / Double(k)) }
        return "\(b) B"
    }

    private func formatEta(_ secs: UInt64) -> String {
        if secs >= 3600 { return "\(secs / 3600)h \((secs % 3600) / 60)m" }
        if secs >= 60   { return "\(secs / 60)m \(secs % 60)s" }
        return "\(secs)s"
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
