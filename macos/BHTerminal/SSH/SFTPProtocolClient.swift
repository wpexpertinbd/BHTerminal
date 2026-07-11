import Foundation

/// A minimal SFTP v3 client that speaks the wire protocol over a pair of
/// pipes (a spawned `ssh -s -- host sftp`'s stdin/stdout). This is what lets
/// the file browser authenticate with **real ssh** — keys, agent,
/// passphrase-protected keys, 2FA, ProxyJump — exactly like the terminal,
/// instead of an in-process SSH library that only did passwords.
///
/// Scope is deliberately just what the browser needs: realpath, directory
/// listing, stat, read/write whole files, mkdir/rmdir/remove/rename, chmod.
/// SFTP v3 is what OpenSSH's server speaks by default.
final class SFTPProtocolClient {
    enum SFTPError: Error, LocalizedError {
        case handshakeFailed
        case unexpectedResponse
        case status(code: UInt32, message: String)
        case connectionClosed
        case timedOut
        case protocolViolation(String)
        case fileTooLarge

        var errorDescription: String? {
            switch self {
            case .handshakeFailed: return "SFTP handshake failed."
            case .unexpectedResponse: return "Unexpected SFTP response."
            case .status(_, let message): return message.isEmpty ? "SFTP error." : message
            case .connectionClosed: return "SFTP connection closed."
            case .timedOut: return "The SFTP server did not respond in time."
            case .protocolViolation(let why): return "SFTP protocol error: \(why)"
            case .fileTooLarge: return "File is too large to transfer in memory."
            }
        }
    }

    // Hostile-server guards. OpenSSH caps SFTP packets at 256 KB; 4 MB is a
    // generous ceiling that still bounds a malicious "huge length" to a safe
    // amount of memory rather than the claimed ~4 GB.
    private static let maxPacketLength = 4 * 1024 * 1024
    /// Cap on a whole-file read held in memory (used by the test/small reads;
    /// real downloads stream to disk via `downloadFile`).
    private static let maxInMemoryFileBytes = 256 * 1024 * 1024
    private static let requestTimeout: TimeInterval = 30
    private static let handshakeTimeout: TimeInterval = 20
    private static let maxDirectoryEntries = 2_000_000

    // Packet types (RFC draft-ietf-secsh-filexfer-02, SFTP v3).
    private enum PktType {
        static let INIT: UInt8 = 1, VERSION: UInt8 = 2
        static let OPEN: UInt8 = 3, CLOSE: UInt8 = 4, READ: UInt8 = 5, WRITE: UInt8 = 6
        static let LSTAT: UInt8 = 7, SETSTAT: UInt8 = 9
        static let OPENDIR: UInt8 = 11, READDIR: UInt8 = 12
        static let REMOVE: UInt8 = 13, MKDIR: UInt8 = 14, RMDIR: UInt8 = 15
        static let REALPATH: UInt8 = 16, STAT: UInt8 = 17, RENAME: UInt8 = 18
        static let STATUS: UInt8 = 101, HANDLE: UInt8 = 102, DATA: UInt8 = 103
        static let NAME: UInt8 = 104, ATTRS: UInt8 = 105
    }

    // Open flags.
    private enum Flags {
        static let READ: UInt32 = 0x1, WRITE: UInt32 = 0x2
        static let CREAT: UInt32 = 0x8, TRUNC: UInt32 = 0x10
    }
    // Attribute flags.
    private enum Attr {
        static let SIZE: UInt32 = 0x1, UIDGID: UInt32 = 0x2, PERMISSIONS: UInt32 = 0x4, ACMODTIME: UInt32 = 0x8, EXTENDED: UInt32 = 0x80000000
    }
    private static let STATUS_OK: UInt32 = 0, STATUS_EOF: UInt32 = 1

    struct Attributes {
        var size: UInt64?
        var permissions: UInt32?
        var modificationTime: Date?
        var isDirectory: Bool
        var isSymlink: Bool
    }

    struct Entry {
        let filename: String
        let longname: String
        let attributes: Attributes
    }

    private let process: Process
    private let writeHandle: FileHandle
    private let readHandle: FileHandle
    private let readerQueue = DispatchQueue(label: "com.biswashost.BHTerminal.sftp.reader")
    private let writeQueue = DispatchQueue(label: "com.biswashost.BHTerminal.sftp.writer")
    private let timeoutQueue = DispatchQueue(label: "com.biswashost.BHTerminal.sftp.timeout")
    private let lock = NSLock()

    private var nextRequestID: UInt32 = 0
    private var pending: [UInt32: CheckedContinuation<(type: UInt8, body: Data), Error>] = [:]
    private var buffer = Data()
    private var closed = false

    private init(process: Process, writeHandle: FileHandle, readHandle: FileHandle) {
        self.process = process
        self.writeHandle = writeHandle
        self.readHandle = readHandle
    }

    /// Launches `executable args…` (an `ssh … -s -- host sftp`) and performs
    /// the SFTP version handshake. Throws if ssh exits before the handshake
    /// (e.g. auth failed / no reusable connection yet) — the caller retries.
    static func launch(executable: String, args: [String]) async throws -> SFTPProtocolClient {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = args
        let stdin = Pipe(), stdout = Pipe()
        process.standardInput = stdin
        process.standardOutput = stdout
        process.standardError = FileHandle.nullDevice // ssh's own stderr chatter isn't ours to show

        let client = SFTPProtocolClient(process: process,
                                        writeHandle: stdin.fileHandleForWriting,
                                        readHandle: stdout.fileHandleForReading)
        try process.run()
        client.startReadLoop()
        do {
            try await client.handshake()
        } catch {
            client.close() // reap the ssh process on a failed/aborted handshake
            throw error
        }
        return client
    }

    func close() {
        lock.lock()
        let alreadyClosed = closed
        closed = true
        let waiters = pending
        pending.removeAll()
        lock.unlock()
        guard !alreadyClosed else { return }
        for (_, cont) in waiters { cont.resume(throwing: SFTPError.connectionClosed) }
        try? writeHandle.close()
        if process.isRunning { process.terminate() }
    }

    // MARK: - Handshake

    private func handshake() async throws {
        var w = ByteWriter()
        w.u8(PktType.INIT)
        w.u32(3) // version
        // INIT has no request id; the VERSION reply also has none, so we can't
        // route it by id — send it and read the next packet directly.
        let reply = try await sendRawExpectingUnkeyed(w.data)
        guard reply.type == PktType.VERSION else { throw SFTPError.handshakeFailed }
    }

    // MARK: - Operations

    func realpath(_ path: String) async throws -> String {
        let (type, body) = try await request(PktType.REALPATH) { $0.string(path) }
        guard type == PktType.NAME else { try throwStatus(type, body); throw SFTPError.unexpectedResponse }
        var r = ByteReader(body)
        _ = r.u32() // request id
        let count = r.u32()
        guard count >= 1 else { throw SFTPError.unexpectedResponse }
        return r.stringUTF8()
    }

    func listDirectory(_ path: String) async throws -> [Entry] {
        let handle = try await openDir(path)
        defer { Task { try? await self.closeHandle(handle) } }
        var entries: [Entry] = []
        while true {
            let (type, body) = try await request(PktType.READDIR) { $0.data(handle) }
            if type == PktType.STATUS {
                var r = ByteReader(body); _ = r.u32()
                let code = r.u32()
                if code == Self.STATUS_EOF { break }
                throw SFTPError.status(code: code, message: r.stringUTF8())
            }
            guard type == PktType.NAME else { throw SFTPError.unexpectedResponse }
            var r = ByteReader(body); _ = r.u32()
            let count = r.u32()
            // `count` is attacker-controlled (up to 4 billion); trust the bytes
            // actually present, not the claimed count, and cap the total so a
            // server streaming endless NAME packets can't grow us without bound.
            var i: UInt32 = 0
            while i < count, !r.isAtEnd {
                let name = r.stringUTF8()
                let longname = r.stringUTF8()
                let attrs = Self.readAttributes(&r, longname: longname)
                entries.append(Entry(filename: name, longname: longname, attributes: attrs))
                i += 1
            }
            guard entries.count <= Self.maxDirectoryEntries else {
                throw SFTPError.protocolViolation("directory listing exceeded \(Self.maxDirectoryEntries) entries")
            }
        }
        return entries
    }

    /// Reads a whole file into memory. Used for small reads/tests; real
    /// downloads use `downloadFile` (streams to disk). Bounded so a never-EOF
    /// server can't OOM the app.
    func readFile(_ path: String) async throws -> Data {
        let handle = try await open(path, pflags: Flags.READ)
        defer { Task { try? await self.closeHandle(handle) } }
        var data = Data()
        var offset: UInt64 = 0
        let chunk: UInt32 = 32_768
        while true {
            let (type, body) = try await request(PktType.READ) { w in
                w.data(handle); w.u64(offset); w.u32(chunk)
            }
            if type == PktType.STATUS {
                var r = ByteReader(body); _ = r.u32()
                let code = r.u32()
                if code == Self.STATUS_EOF { break }
                throw SFTPError.status(code: code, message: r.stringUTF8())
            }
            guard type == PktType.DATA else { throw SFTPError.unexpectedResponse }
            var r = ByteReader(body); _ = r.u32()
            let piece = r.string()
            if piece.isEmpty { break }
            data.append(piece)
            offset += UInt64(piece.count)
            guard data.count <= Self.maxInMemoryFileBytes else { throw SFTPError.fileTooLarge }
        }
        return data
    }

    /// Streams a remote file straight to `fileHandle` — a huge or never-EOF
    /// file can't accumulate in memory this way.
    func downloadFile(_ path: String, to fileHandle: FileHandle) async throws {
        let handle = try await open(path, pflags: Flags.READ)
        defer { Task { try? await self.closeHandle(handle) } }
        var offset: UInt64 = 0
        let chunk: UInt32 = 32_768
        while true {
            let (type, body) = try await request(PktType.READ) { w in
                w.data(handle); w.u64(offset); w.u32(chunk)
            }
            if type == PktType.STATUS {
                var r = ByteReader(body); _ = r.u32()
                let code = r.u32()
                if code == Self.STATUS_EOF { break }
                throw SFTPError.status(code: code, message: r.stringUTF8())
            }
            guard type == PktType.DATA else { throw SFTPError.unexpectedResponse }
            var r = ByteReader(body); _ = r.u32()
            let piece = r.string()
            if piece.isEmpty { break }
            try fileHandle.write(contentsOf: piece)
            offset += UInt64(piece.count)
        }
    }

    func writeFile(_ path: String, data: Data) async throws {
        let handle = try await open(path, pflags: Flags.WRITE | Flags.CREAT | Flags.TRUNC)
        defer { Task { try? await self.closeHandle(handle) } }
        var offset: UInt64 = 0
        let chunk = 32_768
        var index = data.startIndex
        while index < data.endIndex {
            let end = data.index(index, offsetBy: chunk, limitedBy: data.endIndex) ?? data.endIndex
            let slice = Data(data[index..<end])
            try await expectOK(PktType.WRITE) { w in
                w.data(handle); w.u64(offset); w.data(slice)
            }
            offset += UInt64(slice.count)
            index = end
        }
    }

    func makeDirectory(_ path: String) async throws {
        try await expectOK(PktType.MKDIR) { $0.string(path); $0.u32(0) /* no attrs */ }
    }

    func removeDirectory(_ path: String) async throws {
        try await expectOK(PktType.RMDIR) { $0.string(path) }
    }

    func removeFile(_ path: String) async throws {
        try await expectOK(PktType.REMOVE) { $0.string(path) }
    }

    func rename(from: String, to: String) async throws {
        try await expectOK(PktType.RENAME) { $0.string(from); $0.string(to) }
    }

    func setPermissions(_ path: String, mode: UInt32) async throws {
        try await expectOK(PktType.SETSTAT) { w in
            w.string(path)
            w.u32(Attr.PERMISSIONS)
            w.u32(mode)
        }
    }

    // MARK: - Low-level request helpers

    private func openDir(_ path: String) async throws -> Data {
        let (type, body) = try await request(PktType.OPENDIR) { $0.string(path) }
        return try handleOrStatus(type, body)
    }

    private func open(_ path: String, pflags: UInt32) async throws -> Data {
        let (type, body) = try await request(PktType.OPEN) { w in
            w.string(path); w.u32(pflags); w.u32(0) /* no attrs */
        }
        return try handleOrStatus(type, body)
    }

    private func closeHandle(_ handle: Data) async throws {
        try await expectOK(PktType.CLOSE) { $0.data(handle) }
    }

    private func handleOrStatus(_ type: UInt8, _ body: Data) throws -> Data {
        guard type == PktType.HANDLE else { try throwStatus(type, body); throw SFTPError.unexpectedResponse }
        var r = ByteReader(body); _ = r.u32()
        return r.string()
    }

    private func expectOK(_ type: UInt8, build: (inout ByteWriter) -> Void) async throws {
        let (rtype, body) = try await request(type, build: build)
        guard rtype == PktType.STATUS else { throw SFTPError.unexpectedResponse }
        var r = ByteReader(body); _ = r.u32()
        let code = r.u32()
        guard code == Self.STATUS_OK else { throw SFTPError.status(code: code, message: r.stringUTF8()) }
    }

    private func throwStatus(_ type: UInt8, _ body: Data) throws {
        if type == PktType.STATUS {
            var r = ByteReader(body); _ = r.u32()
            let code = r.u32()
            throw SFTPError.status(code: code, message: r.stringUTF8())
        }
    }

    /// Sends a request with a fresh id and awaits the matching reply.
    private func request(_ type: UInt8, build: (inout ByteWriter) -> Void) async throws -> (type: UInt8, body: Data) {
        let id = nextID()
        var w = ByteWriter()
        w.u8(type)
        w.u32(id)
        build(&w)
        return try await withCheckedThrowingContinuation { cont in
            lock.lock()
            if closed { lock.unlock(); cont.resume(throwing: SFTPError.connectionClosed); return }
            pending[id] = cont
            lock.unlock()
            write(framed: w.data)
            scheduleTimeout(id: id, seconds: Self.requestTimeout)
        }
    }

    /// For the id-less INIT/VERSION handshake only.
    private func sendRawExpectingUnkeyed(_ payload: Data) async throws -> (type: UInt8, body: Data) {
        return try await withCheckedThrowingContinuation { cont in
            lock.lock()
            if closed { lock.unlock(); cont.resume(throwing: SFTPError.connectionClosed); return }
            pending[handshakeKey] = cont
            lock.unlock()
            write(framed: payload)
            scheduleTimeout(id: handshakeKey, seconds: Self.handshakeTimeout)
        }
    }
    private let handshakeKey: UInt32 = 0xFFFF_FFFF

    /// Resumes a pending request with `.timedOut` if no reply arrives. The
    /// removeValue-under-lock makes resume exactly-once (a late dispatch or
    /// close finds the id already gone). Without this a stalled server hangs
    /// the operation forever and leaks the ssh child process.
    private func scheduleTimeout(id: UInt32, seconds: TimeInterval) {
        timeoutQueue.asyncAfter(deadline: .now() + seconds) { [weak self] in
            guard let self else { return }
            self.lock.lock()
            let cont = self.pending.removeValue(forKey: id)
            self.lock.unlock()
            cont?.resume(throwing: SFTPError.timedOut)
        }
    }

    private func nextID() -> UInt32 {
        lock.lock(); defer { lock.unlock() }
        nextRequestID &+= 1
        if nextRequestID == handshakeKey { nextRequestID = 1 }
        return nextRequestID
    }

    private func write(framed payload: Data) {
        var out = ByteWriter()
        out.u32(UInt32(payload.count))
        var full = out.data
        full.append(payload)
        // Serialize all writes off the caller's thread so two concurrent
        // operations can't interleave bytes and desync the framed stream.
        writeQueue.async { [weak self] in
            guard let self else { return }
            do { try self.writeHandle.write(contentsOf: full) }
            catch { self.failAll(SFTPError.connectionClosed) }
        }
    }

    // MARK: - Read loop

    private func startReadLoop() {
        readerQueue.async { [weak self] in self?.readLoop() }
    }

    private func readLoop() {
        while true {
            let chunk = readHandle.availableData
            if chunk.isEmpty { failAll(SFTPError.connectionClosed); return }
            buffer.append(chunk)
            drainPackets()
        }
    }

    private func drainPackets() {
        var cursor = 0
        while buffer.count - cursor >= 4 {
            let len = Int(u32(buffer, at: cursor))
            if len < 1 || len > Self.maxPacketLength {
                // Hostile/garbled length — tear down rather than buffer toward a
                // ~4 GB claim or wedge forever on a zero-length frame.
                failAll(SFTPError.protocolViolation("packet length \(len) out of range"))
                return
            }
            guard buffer.count - cursor - 4 >= len else { break }
            let start = cursor + 4
            let type = buffer[buffer.startIndex + start]
            let body = Data(buffer[(buffer.startIndex + start + 1)..<(buffer.startIndex + start + len)])
            cursor = start + len
            dispatch(type: type, body: body)
        }
        if cursor > 0 { buffer.removeSubrange(buffer.startIndex..<(buffer.startIndex + cursor)) }
    }

    private func dispatch(type: UInt8, body: Data) {
        // VERSION (handshake) has no request id; everything else's id is the
        // first 4 bytes of the body.
        let key: UInt32
        if type == PktType.VERSION {
            key = handshakeKey
        } else if body.count >= 4 {
            key = u32(body, at: 0)
        } else {
            return
        }
        lock.lock()
        let cont = pending.removeValue(forKey: key)
        lock.unlock()
        cont?.resume(returning: (type, body))
    }

    private func failAll(_ error: Error) {
        lock.lock()
        closed = true
        let waiters = pending
        pending.removeAll()
        lock.unlock()
        for (_, cont) in waiters { cont.resume(throwing: error) }
    }

    // MARK: - Byte helpers

    private func u32(_ data: Data, at offset: Int) -> UInt32 {
        let b = data.startIndex + offset
        return (UInt32(data[b]) << 24) | (UInt32(data[b + 1]) << 16) | (UInt32(data[b + 2]) << 8) | UInt32(data[b + 3])
    }

    private static func readAttributes(_ r: inout ByteReader, longname: String) -> Attributes {
        let flags = r.u32()
        var size: UInt64?, perms: UInt32?, mtime: Date?
        if flags & Attr.SIZE != 0 { size = r.u64() }
        if flags & Attr.UIDGID != 0 { _ = r.u32(); _ = r.u32() }
        if flags & Attr.PERMISSIONS != 0 { perms = r.u32() }
        if flags & Attr.ACMODTIME != 0 { _ = r.u32(); let m = r.u32(); mtime = Date(timeIntervalSince1970: TimeInterval(m)) }
        if flags & Attr.EXTENDED != 0 {
            let count = r.u32()
            var i: UInt32 = 0
            // `count` is attacker-controlled; stop at the real end of the
            // buffer instead of spinning up to 4 billion times.
            while i < count, !r.isAtEnd { _ = r.string(); _ = r.string(); i += 1 }
        }
        let isDir: Bool, isLink: Bool
        if let mode = perms {
            isDir = (mode & 0o170000) == 0o040000
            isLink = (mode & 0o170000) == 0o120000
        } else {
            isDir = longname.first == "d"
            isLink = longname.first == "l"
        }
        return Attributes(size: size, permissions: perms, modificationTime: mtime, isDirectory: isDir, isSymlink: isLink)
    }
}

// MARK: - Byte reader/writer

private struct ByteWriter {
    private(set) var data = Data()
    mutating func u8(_ v: UInt8) { data.append(v) }
    mutating func u32(_ v: UInt32) { withUnsafeBytes(of: v.bigEndian) { data.append(contentsOf: $0) } }
    mutating func u64(_ v: UInt64) { withUnsafeBytes(of: v.bigEndian) { data.append(contentsOf: $0) } }
    mutating func string(_ s: String) { let b = Array(s.utf8); u32(UInt32(b.count)); data.append(contentsOf: b) }
    mutating func data(_ d: Data) { u32(UInt32(d.count)); data.append(d) }
}

private struct ByteReader {
    private let data: Data
    private var offset: Int
    init(_ d: Data) { data = d; offset = 0 }

    var isAtEnd: Bool { offset >= data.count }

    mutating func u32() -> UInt32 {
        guard offset + 4 <= data.count else { offset = data.count; return 0 }
        let b = data.startIndex + offset
        let v = (UInt32(data[b]) << 24) | (UInt32(data[b + 1]) << 16) | (UInt32(data[b + 2]) << 8) | UInt32(data[b + 3])
        offset += 4
        return v
    }
    mutating func u64() -> UInt64 {
        (UInt64(u32()) << 32) | UInt64(u32())
    }
    mutating func string() -> Data {
        let n = Int(u32())
        guard n >= 0, offset + n <= data.count else { offset = data.count; return Data() }
        let b = data.startIndex + offset
        let sub = Data(data[b..<(b + n)])
        offset += n
        return sub
    }
    mutating func stringUTF8() -> String { String(decoding: string(), as: UTF8.self) }
}
