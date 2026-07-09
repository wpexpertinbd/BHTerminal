import SwiftUI

@main
struct BHTerminalApp: App {
    var body: some Scene {
        WindowGroup {
            MainWindowView()
        }
        .defaultSize(width: 1280, height: 800)
        .windowToolbarStyle(.unified)
    }
}
