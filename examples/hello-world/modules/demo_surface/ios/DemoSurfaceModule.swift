import SwiftUI
import UIKit

final class AtomHostRootViewProviderImpl: AtomHostRootViewProvider {
    override func makeRootViewController() -> UIViewController {
        UIHostingController(rootView: HelloWorldDemoRootView())
    }
}

private enum HelloWorldDemoElement: String {
    case title = "atom.demo.title"
    case slug = "atom.demo.slug"
    case status = "atom.demo.status"
    case input = "atom.demo.input"
    case button = "atom.demo.primary_button"
    case echo = "atom.demo.echo"
}

private final class HelloWorldDemoState: ObservableObject {
    @Published var inputText = ""
    @Published var statusText = "ready"
    @Published var echoText = "Typed: "
    @Published var tapCount = 0
}

private struct HelloWorldDemoRootView: View {
    @StateObject private var state = HelloWorldDemoState()

    var body: some View {
        ZStack {
            Color(uiColor: .systemBackground)
                .ignoresSafeArea()
            VStack(spacing: 16) {
                Text("Hello Atom")
                    .font(.title2.weight(.semibold))
                    .accessibilityIdentifier(HelloWorldDemoElement.title.rawValue)
                Text("hello-atom")
                    .font(.footnote.monospaced())
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier(HelloWorldDemoElement.slug.rawValue)
                Text(state.statusText)
                    .font(.subheadline.monospaced())
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier(HelloWorldDemoElement.status.rawValue)
                TextField("Type something", text: $state.inputText)
                    .textFieldStyle(.roundedBorder)
                    .padding(.horizontal, 24)
                    .accessibilityIdentifier(HelloWorldDemoElement.input.rawValue)
                    .onChange(of: state.inputText) {
                        state.echoText = "Typed: \(state.inputText)"
                        state.statusText = "typed-text"
                    }
                Button("Primary Action") {
                    state.tapCount += 1
                    state.statusText = "primary-button-tapped-\(state.tapCount)"
                }
                .buttonStyle(.borderedProminent)
                .accessibilityIdentifier(HelloWorldDemoElement.button.rawValue)
                Text(state.echoText)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier(HelloWorldDemoElement.echo.rawValue)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .padding(.top, 96)
        }
    }
}
