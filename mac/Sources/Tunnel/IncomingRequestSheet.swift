import SwiftUI

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
                .buttonStyle(.borderedProminent)
            }
        }
        .padding(32)
        .frame(width: 360)
    }
}
