import Foundation

/// A port-forwarding rule attached to a Host, translated straight into
/// `ssh -L/-R/-D` flags by TunnelManager rather than reimplemented.
struct TunnelRule: Identifiable, Codable, Hashable {
    enum Kind: String, Codable {
        case local    // ssh -L listenPort:destHost:destPort
        case remote   // ssh -R listenPort:destHost:destPort
        case dynamic  // ssh -D listenPort (SOCKS proxy)
    }

    var id: UUID = UUID()
    var name: String = ""
    var kind: Kind
    var listenHost: String = "127.0.0.1"
    var listenPort: Int
    var destHost: String = ""
    var destPort: Int = 0
    var enabled: Bool = true
}
