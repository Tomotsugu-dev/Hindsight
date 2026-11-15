// swift-tools-version:5.9
// build.rs 用 swift-rs 编译这个包，再把它静态链接进 app。

import PackageDescription

let package = Package(
    name: "AppleFm",
    platforms: [.macOS(.v14)],
    products: [
        // 名字要和 build.rs 里 `with_package` 的名字一样，swift-rs 按它找静态库。
        .library(name: "AppleFm", type: .static, targets: ["AppleFm"])
    ],
    targets: [
        .target(name: "AppleFm")
    ]
)
