import AtomRuntimeBridge
import UIKit

final class AtomAppDelegate: NSObject, UIApplicationDelegate {
    private var handle: AtomRuntimeHandle = 0

    func application(
        _: UIApplication,
        didFinishLaunchingWithOptions _: [UIApplication.LaunchOptionsKey: Any]? = nil,
    ) -> Bool {
        true
    }

    func application(
        _: UIApplication,
        configurationForConnecting connectingSceneSession: UISceneSession,
        options _: UIScene.ConnectionOptions,
    ) -> UISceneConfiguration {
        let configuration = UISceneConfiguration(
            name: "Default Configuration",
            sessionRole: connectingSceneSession.role,
        )
        configuration.delegateClass = AtomSceneDelegate.self
        return configuration
    }

    func initializeRuntime() {
        guard handle == 0 else {
            return
        }
        do {
            var handle: AtomRuntimeHandle = 0
            var errorBuffer = AtomOwnedBuffer(ptr: nil, len: 0, cap: 0)
            let status = atom_app_init(AtomSlice(ptr: nil, len: 0), &handle, &errorBuffer)
            defer { freeBuffer(&errorBuffer) }
            try ensureSuccess(status, action: "atom_app_init")
            self.handle = handle
        } catch {
            logError(error, action: "launch")
        }
    }

    func sendLifecycle(_ event: UInt32, action: String) {
        guard handle != 0 else {
            return
        }
        do {
            var errorBuffer = AtomOwnedBuffer(ptr: nil, len: 0, cap: 0)
            let status = atom_app_handle_lifecycle(handle, event, &errorBuffer)
            defer { freeBuffer(&errorBuffer) }
            try ensureSuccess(status, action: action)
        } catch {
            logError(error, action: action)
        }
    }

    func shutdownRuntime() {
        guard handle != 0 else {
            return
        }
        atom_app_shutdown(handle)
        handle = 0
    }

    private func ensureSuccess(_ status: Int32, action: String) throws {
        if status == 0 {
            return
        }
        throw NSError(domain: "AtomRuntime", code: Int(status), userInfo: [NSLocalizedDescriptionKey: "\(action) failed with status \(status)"])
    }

    private func freeBuffer(_ buffer: inout AtomOwnedBuffer) {
        guard buffer.ptr != nil else {
            return
        }
        atom_buffer_free(buffer)
        buffer = AtomOwnedBuffer(ptr: nil, len: 0, cap: 0)
    }

    private func logError(_ error: Error, action: String) {
        NSLog("Atom iOS host %@ failed: %@", action, String(describing: error))
    }
}
