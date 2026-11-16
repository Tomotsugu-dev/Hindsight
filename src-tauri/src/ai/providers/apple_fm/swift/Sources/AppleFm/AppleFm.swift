// 把系统的 Apple Intelligence 模型包成 C 函数，给 Rust 调用。
// 这里只包系统调用，判断都写在 Rust 里（`apple_fm/mod.rs`）。
// 同步函数交给 Rust 的字符串用 `strdup` 分配，Rust 读完调 `hs_afm_free` 释放。
// 异步函数马上返回任务号，做完调 Rust 给的回调，每个任务一次，取消时也调。

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

/// Rust 给的回调。`context` 原样交回。`json` 只在回调期间有效。
public typealias HsAfmCallback = @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>) -> Void

/// 数 instructions 和 prompt 一共多少 token。马上返回任务号，数完调 `callback`。
/// macOS 26.4 以前系统数不了，交回的结果里没有 `tokens`。
@_cdecl("hs_afm_count_tokens")
public func hs_afm_count_tokens(
    _ instructions: UnsafePointer<CChar>,
    _ prompt: UnsafePointer<CChar>,
    _ context: UnsafeMutableRawPointer?,
    _ callback: HsAfmCallback
) -> UInt64 {
    let instructions = String(cString: instructions)
    let prompt = String(cString: prompt)
    let reply = Reply(context: context, callback: callback)
    return running.start {
        guard #available(macOS 26.4, *) else {
            reply.send(Outcome())
            return
        }
        let model = SystemLanguageModel(guardrails: .permissiveContentTransformations)
        do {
            let forInstructions = try await model.tokenCount(for: instructions)
            let forPrompt = try await model.tokenCount(for: prompt)
            reply.send(Outcome(tokens: forInstructions + forPrompt))
        } catch {
            reply.send(Outcome(error: SystemError(error)))
        }
    }
}

/// 新建一个 session，用 `instructions` 当系统提示词，回答 `prompt`。马上返回任务号，做完调 `callback`。
/// `temperature` 是 NaN 时用系统默认。
@_cdecl("hs_afm_generate")
public func hs_afm_generate(
    _ instructions: UnsafePointer<CChar>,
    _ prompt: UnsafePointer<CChar>,
    _ maxTokens: UInt32,
    _ temperature: Double,
    _ context: UnsafeMutableRawPointer?,
    _ callback: HsAfmCallback
) -> UInt64 {
    let instructions = String(cString: instructions)
    let prompt = String(cString: prompt)
    let reply = Reply(context: context, callback: callback)
    return running.start {
        guard #available(macOS 26.0, *) else {
            reply.send(Outcome(error: SystemError(domain: "hindsight.apple_fm", code: 0, message: "needs macOS 26")))
            return
        }
        let model = SystemLanguageModel(guardrails: .permissiveContentTransformations)
        let session = LanguageModelSession(model: model, instructions: instructions)
        let options = GenerationOptions(
            temperature: temperature.isNaN ? nil : temperature,
            maximumResponseTokens: Int(maxTokens)
        )
        do {
            let response = try await session.respond(to: prompt, options: options)
            reply.send(Outcome(text: response.content))
        } catch {
            reply.send(Outcome(error: SystemError(error)))
        }
    }
}

/// 取消任务。任务已经做完时什么也不做。取消的任务照样调一次回调，结果是 `CancellationError`。
@_cdecl("hs_afm_cancel")
public func hs_afm_cancel(_ task: UInt64) {
    running.cancel(task)
}

/// 交给 Rust 的结果，对应 Rust 的 `Outcome`。
private struct Outcome: Encodable {
    var text: String? = nil
    var tokens: Int? = nil
    var error: SystemError? = nil
}

/// 系统报的错误，对应 Rust 的 `SystemError`。Rust 按 `domain` 和 `code` 认是哪种错误。
private struct SystemError: Encodable {
    let domain: String
    let code: Int
    let message: String

    init(domain: String, code: Int, message: String) {
        self.domain = domain
        self.code = code
        self.message = message
    }

    init(_ error: Error) {
        let ns = error as NSError
        self.init(domain: ns.domain, code: ns.code, message: error.localizedDescription)
    }
}

/// 把结果交给 Rust 的回调。每个任务只调一次。
private struct Reply: @unchecked Sendable {
    let context: UnsafeMutableRawPointer?
    let callback: HsAfmCallback

    func send(_ outcome: Outcome) {
        let data = (try? JSONEncoder().encode(outcome)) ?? Data("{}".utf8)
        String(decoding: data, as: UTF8.self).withCString { callback(context, $0) }
    }
}

/// 在跑的任务，按任务号取消。
private final class Running: @unchecked Sendable {
    private let lock = NSLock()
    private var next: UInt64 = 0
    private var tasks: [UInt64: Task<Void, Never>] = [:]

    /// 起一个任务，返回任务号。任务做完把自己从表里删掉。
    func start(_ body: @escaping @Sendable () async -> Void) -> UInt64 {
        // 拿着锁放进表：任务做得再快，也要等放进去以后才能删。
        lock.lock()
        defer { lock.unlock() }
        next += 1
        let id = next
        tasks[id] = Task {
            await body()
            self.remove(id)
        }
        return id
    }

    func cancel(_ id: UInt64) {
        lock.lock()
        let task = tasks[id]
        lock.unlock()
        task?.cancel()
    }

    private func remove(_ id: UInt64) {
        lock.lock()
        tasks[id] = nil
        lock.unlock()
    }
}

private let running = Running()

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
