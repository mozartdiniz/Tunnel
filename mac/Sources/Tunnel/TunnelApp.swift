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

        Settings {
            SettingsView()
                .environmentObject(model)
        }
    }
}

// MARK: - SettingsView

struct SettingsView: View {
    @EnvironmentObject var model: AppModel
    @State private var deviceName: String = ""
    @State private var downloadDir: URL = FileManager.default
        .urls(for: .downloadsDirectory, in: .userDomainMask)[0]

    var body: some View {
        Form {
            Section("Device") {
                TextField("Device name", text: $deviceName)
                    .onSubmit { model.updateDeviceName(deviceName) }
            }

            Section("Downloads") {
                HStack {
                    Text(downloadDir.path)
                        .truncationMode(.middle)
                        .lineLimit(1)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Choose…") { chooseDownloadDir() }
                }
            }
        }
        .formStyle(.grouped)
        .padding()
        .frame(width: 400)
        .onAppear {
            deviceName = model.deviceName
            downloadDir = Config.load().downloadDir
        }
    }

    private func chooseDownloadDir() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "Select"
        if panel.runModal() == .OK, let url = panel.url {
            downloadDir = url
            model.updateDownloadDir(url)
        }
    }
}
