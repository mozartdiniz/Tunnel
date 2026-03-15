import SwiftUI

@main
struct TunnelApp: App {
    @StateObject private var model = AppModel()

    init() {
        // When launched from Terminal (swift run), the app doesn't automatically
        // become frontmost. Force it to the foreground.
        DispatchQueue.main.async {
            NSApplication.shared.setActivationPolicy(.regular)
            NSApplication.shared.activate(ignoringOtherApps: true)
        }
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(model)
                .task {
                    await model.start()
                }
        }
        .windowStyle(.titleBar)
        .windowToolbarStyle(.unified)
        .commands {
            CommandGroup(replacing: .newItem) {}  // Remove File > New
        }

    }
}

// MARK: - SettingsView

struct SettingsView: View {
    @EnvironmentObject var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var deviceName: String = ""
    @State private var downloadDir: URL = FileManager.default
        .urls(for: .downloadsDirectory, in: .userDomainMask)[0]

    var body: some View {
        VStack(spacing: 0) {
        HStack {
            Text("Settings")
                .font(.headline)
            Spacer()
            Button("Done") { dismiss() }
                .keyboardShortcut(.return)
        }
        .padding([.horizontal, .top], 20)
        .padding(.bottom, 12)
        }
        Form {
            Section("Device") {
                TextField("Device name", text: $deviceName)
                    .onSubmit { save() }
                    .onChange(of: deviceName, perform: { _ in save() })
                Text("This name is visible to other devices on the network.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Section("Downloads") {
                HStack {
                    Text(downloadDir.abbreviatingWithTildeInPath)
                        .truncationMode(.middle)
                        .lineLimit(1)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Choose…") { chooseDownloadDir() }
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 420)
        .onAppear {
            deviceName = model.deviceName
            downloadDir = model.downloadDir
        }
    }

    private func save() {
        let trimmed = deviceName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        model.updateDeviceName(trimmed)
    }

    private func chooseDownloadDir() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.directoryURL = downloadDir
        panel.prompt = "Select"
        if panel.runModal() == .OK, let url = panel.url {
            downloadDir = url
            model.updateDownloadDir(url)
        }
    }
}

private extension URL {
    var abbreviatingWithTildeInPath: String {
        (path as NSString).abbreviatingWithTildeInPath
    }
}
