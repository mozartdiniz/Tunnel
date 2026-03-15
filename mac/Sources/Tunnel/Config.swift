import Foundation

struct Config: Codable {
    var deviceName: String
    var downloadDir: URL

    static func load() -> Config {
        guard let data = try? Data(contentsOf: configFileURL()),
              let config = try? JSONDecoder().decode(Config.self, from: data)
        else { return Config() }
        return config
    }

    func save() {
        guard let data = try? JSONEncoder().encode(self) else { return }
        try? FileManager.default.createDirectory(at: Self.dataDir(), withIntermediateDirectories: true)
        try? data.write(to: Self.configFileURL())
    }

    static func dataDir() -> URL {
        FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Tunnel", isDirectory: true)
    }

    private static func configFileURL() -> URL {
        dataDir().appendingPathComponent("config.json")
    }
}

extension Config {
    init() {
        deviceName = Host.current().localizedName ?? ProcessInfo.processInfo.hostName
        downloadDir = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask)[0]
    }
}
