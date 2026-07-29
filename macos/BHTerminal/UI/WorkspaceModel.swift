import Foundation

/// The open tabs and which one is selected, owned at App scope (like
/// SessionStore and TunnelManager) rather than by the window's view.
///
/// Two reasons it can't be view `@State`:
///  1. Closing the window drops the app to the menu bar and destroys the
///     window's views. With the tab list in view state, the live sessions
///     (owned by TerminalSessionRegistry) would be left running with no tab
///     referencing them — invisible, unkillable ssh processes.
///  2. Re-opening from the menu bar now restores the sessions you had open,
///     which is what "keeps running in the background" should mean.
@MainActor
@Observable
final class WorkspaceModel {
    var tabs: [WorkspaceTab] = []
    var selectedTabID: UUID?
}
