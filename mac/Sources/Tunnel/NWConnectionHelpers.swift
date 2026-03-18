import Foundation
import Network

// MARK: - NWConnection async helpers

func receiveChunk(_ conn: NWConnection, maxLength: Int) async throws -> Data {
    try await withCheckedThrowingContinuation { cont in
        conn.receive(minimumIncompleteLength: 1, maximumLength: maxLength) { data, _, isComplete, error in
            if let error { cont.resume(throwing: error) }
            else if let data, !data.isEmpty { cont.resume(returning: data) }
            else if isComplete { cont.resume(throwing: NWError.posix(.ECONNRESET)) }
            else { cont.resume(throwing: NWError.posix(.EAGAIN)) }
        }
    }
}

func readHTTPHeaders(_ conn: NWConnection) async throws
    -> (method: String, path: String, headers: [String: String], bodyStart: Data)
{
    var buffer = Data()
    let delimiter = Data("\r\n\r\n".utf8)

    while buffer.count < 65536 {
        let chunk = try await receiveChunk(conn, maxLength: 4096)
        buffer.append(chunk)
        if let range = buffer.range(of: delimiter) {
            let headerBytes = buffer[..<range.lowerBound]
            let bodyStart   = Data(buffer[range.upperBound...])
            let headerStr   = String(decoding: headerBytes, as: UTF8.self)
            let lines       = headerStr.components(separatedBy: "\r\n")
            guard let requestLine = lines.first else { throw HTTPError.badRequest }
            let parts = requestLine.split(separator: " ", maxSplits: 2)
            guard parts.count >= 2 else { throw HTTPError.badRequest }
            var headers: [String: String] = [:]
            for line in lines.dropFirst() {
                guard let colonIdx = line.firstIndex(of: ":") else { continue }
                let key = String(line[..<colonIdx]).trimmingCharacters(in: .whitespaces).lowercased()
                let val = String(line[line.index(after: colonIdx)...]).trimmingCharacters(in: .whitespaces)
                headers[key] = val
            }
            return (String(parts[0]), String(parts[1]), headers, bodyStart)
        }
    }
    throw HTTPError.requestTooLarge
}

func readBody(_ conn: NWConnection, alreadyRead: Data, remaining: Int) async throws -> Data {
    var body = alreadyRead
    var left = remaining
    while left > 0 {
        let chunk = try await receiveChunk(conn, maxLength: min(65536, left))
        body.append(chunk)
        left -= chunk.count
    }
    return body
}

func sendHTTPResponse(
    _ conn: NWConnection, status: Int,
    headers: [String: String] = [:], body: Data
) async throws {
    var lines = ["HTTP/1.1 \(status) \(httpStatusText(status))"]
    lines.append("Content-Length: \(body.count)")
    lines.append("Connection: close")
    for (k, v) in headers { lines.append("\(k): \(v)") }
    lines.append("")
    lines.append("")
    var response = Data(lines.joined(separator: "\r\n").utf8)
    response.append(body)
    try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
        conn.send(content: response, completion: .contentProcessed { error in
            if let e = error { cont.resume(throwing: e) } else { cont.resume() }
        })
    }
    conn.cancel()
}

func parseQueryString(_ query: String) -> [String: String] {
    var result: [String: String] = [:]
    for item in query.split(separator: "&") {
        let kv = item.split(separator: "=", maxSplits: 1)
        guard kv.count == 2 else { continue }
        let k = String(kv[0]).removingPercentEncoding ?? String(kv[0])
        let v = String(kv[1]).removingPercentEncoding ?? String(kv[1])
        result[k] = v
    }
    return result
}

func httpStatusText(_ code: Int) -> String {
    switch code {
    case 200: return "OK"
    case 400: return "Bad Request"
    case 403: return "Forbidden"
    case 404: return "Not Found"
    case 413: return "Content Too Large"
    case 500: return "Internal Server Error"
    case 507: return "Insufficient Storage"
    default:  return "Unknown"
    }
}

// MARK: - HTTP error types

enum HTTPError: Error {
    case badRequest
    case requestTooLarge
}
