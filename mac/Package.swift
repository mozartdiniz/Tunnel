// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "Tunnel",
    platforms: [.macOS(.v13)],
    dependencies: [
        .package(url: "https://github.com/apple/swift-certificates.git", from: "1.0.0"),
    ],
    targets: [
        .executableTarget(
            name: "Tunnel",
            dependencies: [
                .product(name: "X509", package: "swift-certificates"),
            ],
            path: "Sources/Tunnel"
        )
    ]
)
