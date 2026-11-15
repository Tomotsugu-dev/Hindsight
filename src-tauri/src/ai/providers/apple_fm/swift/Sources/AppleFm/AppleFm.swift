// 把系统的 Apple Intelligence 模型包成 C 函数，给 Rust 调用。
// 这里只包系统调用，判断都写在 Rust 里（`apple_fm/mod.rs`）。
// 交给 Rust 的字符串用 `strdup` 分配，Rust 读完调 `hs_afm_free` 释放。

import Foundation
import FoundationModels

/// 交给 Rust 的状态，对应 Rust 的 `SystemStatus`。
private struct Status: Encodable {
    /// `os_too_old`、`available`、`device_not_eligible`、`apple_intelligence_not_enabled`、
    /// `model_not_ready`，或者系统新加的原因 `unknown`。
    let availability: String
    let languageSupported: Bool
    let contextSize: Int?
}

/// 查这台 Mac 能不能用模型，以及支不支持 `language`（BCP 47 语言标签）。返回 JSON。
@_cdecl("hs_afm_status")
public func hs_afm_status(_ language: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>? {
    guard #available(macOS 26.0, *) else {
        return encode(Status(availability: "os_too_old", languageSupported: false, contextSize: nil))
    }
    let model = SystemLanguageModel(guardrails: .permissiveContentTransformations)
    switch model.availability {
    case .available:
        let supported = model.supportsLocale(Locale(identifier: String(cString: language)))
        return encode(Status(availability: "available", languageSupported: supported, contextSize: model.contextSize))
    case .unavailable(let reason):
        return encode(Status(availability: name(of: reason), languageSupported: false, contextSize: nil))
    }
}

@_cdecl("hs_afm_free")
public func hs_afm_free(_ string: UnsafeMutablePointer<CChar>?) {
    free(string)
}

@available(macOS 26.0, *)
private func name(of reason: SystemLanguageModel.Availability.UnavailableReason) -> String {
    switch reason {
    case .deviceNotEligible: return "device_not_eligible"
    case .appleIntelligenceNotEnabled: return "apple_intelligence_not_enabled"
    case .modelNotReady: return "model_not_ready"
    @unknown default: return "unknown"
    }
}

private func encode<T: Encodable>(_ value: T) -> UnsafeMutablePointer<CChar>? {
    guard let data = try? JSONEncoder().encode(value) else { return nil }
    return strdup(String(decoding: data, as: UTF8.self))
}
