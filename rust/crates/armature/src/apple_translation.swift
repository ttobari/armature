import AppKit
import Foundation
import NaturalLanguage
import Translation

public typealias CockpitTranslationCallback = @convention(c) (
    UnsafeMutableRawPointer?, UnsafePointer<CChar>?
) -> Void

// `TranslationSession(installedSource:target:)` は macOS 26 から。15〜15.x では
// 翻訳だけ使えない(アプリの他の部分は動く)。
@available(macOS 26.0, *)
@MainActor
private final class CockpitTranslator {
    static let shared = CockpitTranslator()

    // 訳す向き(元の言語 → 先の言語)ごとに1本ずつ持つ。向きが変われば開き直す。
    private var pair: (source: String, target: String)?
    private var session: TranslationSession?
    private var cache: [String: String] = [:]

    func translate(_ texts: [String], from source: String, to target: String) async throws -> [String] {
        if pair?.source != source || pair?.target != target {
            pair = (source, target)
            session = nil
            cache = [:]
        }
        var output = [String](repeating: "", count: texts.count)
        var pending: [(index: Int, text: String)] = []
        for (index, text) in texts.enumerated() {
            if let translated = cache[text] {
                output[index] = translated
            } else {
                pending.append((index, text))
            }
        }
        if pending.isEmpty { return output }

        let active = session ?? TranslationSession(
            installedSource: Locale.Language(identifier: source),
            target: Locale.Language(identifier: target)
        )
        session = active
        let requests = pending.enumerated().map {
            TranslationSession.Request(
                sourceText: $0.element.text,
                clientIdentifier: String($0.offset)
            )
        }
        do {
            for try await response in active.translate(batch: requests) {
                guard let identifier = response.clientIdentifier,
                      let slot = Int(identifier), slot < pending.count else { continue }
                let item = pending[slot]
                output[item.index] = response.targetText
                cache[item.text] = response.targetText
            }
        } catch {
            session = nil
            throw error
        }
        return output
    }
}

private func reply(
    _ context: UnsafeMutableRawPointer?,
    _ callback: @escaping CockpitTranslationCallback,
    _ value: [String: Any]
) {
    let data = try? JSONSerialization.data(withJSONObject: value)
    let json = data.flatMap { String(data: $0, encoding: .utf8) }
        ?? "{\"ok\":false,\"error\":\"can't encode the reply\"}"
    json.withCString { callback(context, $0) }
}

/// `target` は訳す先の言語("en" か "ja")。元の言語は本文から見立てる——
/// 先の言語と同じなら訳さずに返す(日本語の頁を日本語へ「訳し直して」壊さない)。
@_cdecl("cockpit_apple_translate")
public func cockpitAppleTranslate(
    _ input: UnsafePointer<CChar>?,
    _ target: UnsafePointer<CChar>?,
    _ context: UnsafeMutableRawPointer?,
    _ callback: @escaping CockpitTranslationCallback
) {
    let target = target.map { String(cString: $0) } ?? "ja"
    let english = target != "ja"
    guard let input,
          let data = String(cString: input).data(using: .utf8),
          let texts = try? JSONDecoder().decode([String].self, from: data),
          !texts.isEmpty else {
        reply(context, callback, ["ok": false, "error": "the text to translate is broken"])
        return
    }

    let sample = String(texts.prefix(40).joined(separator: " ").prefix(2000))
    let recognizer = NLLanguageRecognizer()
    recognizer.processString(sample)
    let source = recognizer.dominantLanguage?.rawValue ?? (english ? "und" : "en")
    if source == target || source.hasPrefix(target + "-") {
        reply(context, callback, ["ok": true, "lang": target, "texts": texts])
        return
    }
    if source == "und" {
        reply(context, callback, ["ok": false, "error": english
            ? "Couldn't tell the page's language"
            : "頁の言語を見立てられなかった"])
        return
    }

    if #available(macOS 26.0, *) {
        Task { @MainActor in
            do {
                let translated = try await CockpitTranslator.shared.translate(texts, from: source, to: target)
                reply(context, callback, ["ok": true, "lang": source, "texts": translated])
            } catch {
                reply(context, callback, ["ok": false, "error": String(describing: error)])
            }
        }
    } else {
        reply(context, callback, ["ok": false, "error": english
            ? "Page translation needs macOS 26 or later (Claude's translation with ⇧T works on any version)"
            : "頁の翻訳は macOS 26 以降で使えます(⇧T の Claude 精訳はどの版でも使えます)"])
    }
}

// MARK: - クリップボードの画像

// Claude Code は Ctrl+V を受けると自分でクリップボードの画像を読む。こちらは
// 「いま画像を持っているか」だけ答え、貼るのは端末へ Ctrl+V を送って任せる
// ——パスの字を流し込むより、利用者が Ctrl+V を押したときと同じ道になる。
@_cdecl("cockpit_clipboard_has_image")
public func cockpit_clipboard_has_image() -> Bool {
    let pasteboard = NSPasteboard.general
    if pasteboard.data(forType: .png) != nil || pasteboard.data(forType: .tiff) != nil {
        return true
    }
    guard
        let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: nil) as? [URL]
    else {
        return false
    }
    let imageSuffixes = ["png", "jpg", "jpeg", "gif", "webp", "heic", "bmp", "tiff"]
    return urls.contains { imageSuffixes.contains($0.pathExtension.lowercased()) }
}

// MARK: - クリップボードの字

// **⌘V が来た瞬間に、その場で読む。** iced の `clipboard::read()` は非同期で、
// 答えが返るのは次の周回になる。音声入力のアプリは ⌘V を送った直後に
// 元のクリップボードへ戻すので、周回を1つ待つと**戻されたあとの中身を
// 読んでしまい、何も貼られない**。
//
// 返す領域は `strdup` で確保する。呼び手(Rust)が `cockpit_free_string` で返す。
@_cdecl("cockpit_clipboard_text")
public func cockpit_clipboard_text() -> UnsafeMutablePointer<CChar>? {
    guard let text = NSPasteboard.general.string(forType: .string), !text.isEmpty else {
        return nil
    }
    return strdup(text)
}

@_cdecl("cockpit_free_string")
public func cockpit_free_string(_ pointer: UnsafeMutablePointer<CChar>?) {
    guard let pointer else { return }
    free(pointer)
}
