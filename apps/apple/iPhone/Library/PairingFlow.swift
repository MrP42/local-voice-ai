import AVFoundation
import SwiftUI
import UIKit
import VisionKit

/// Geräte-Kopplung: QR-Code am PC scannen, das iPhone ist danach ohne Passwort
/// angemeldet (Spec 2026-11-05-geraete-kopplung-qr, Abschnitt 9). Das Protokoll
/// steht in VoiceCore/Pairing.swift; hier sind nur Netz, Zustand und Oberfläche.
private struct HubTransport: PairingTransport {
    let base: String

    private func send(_ request: URLRequest) async throws -> (Int, Data) {
        let (data, response) = try await URLSession.shared.data(for: request, delegate: NoRedirects.shared)
        return ((response as? HTTPURLResponse)?.statusCode ?? 0, data)
    }

    private func request(_ path: String, pollToken: String?) throws -> URLRequest {
        guard let url = URL(string: base + path) else { throw PairingError.invalidCode }
        var request = URLRequest(url: url, timeoutInterval: 15)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if let pollToken { request.setValue(pollToken, forHTTPHeaderField: "X-Pairing-Token") }
        return request
    }

    func post(_ path: String, body: [String: String], pollToken: String?) async throws -> (Int, Data) {
        var request = try request(path, pollToken: pollToken)
        request.httpMethod = "POST"
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        return try await send(request)
    }

    func get(_ path: String, pollToken: String) async throws -> (Int, Data) {
        try await send(try request(path, pollToken: pollToken))
    }
}

@MainActor
final class PairingFlow: ObservableObject {
    static let shared = PairingFlow()

    enum Phase: Equatable {
        case scan
        /// Vor dem ersten Schritt zum Hub: Link von außen oder fremder Server.
        case ready(PairingCode, viaLink: Bool)
        case running(PairingStage)
        /// Nur nach einem Link: Ein Link kann auch von einer fremden Seite kommen.
        case confirmAccount(String)
        case done(email: String, confirmed: Bool)
        case failed(String)
    }

    @Published private(set) var phase = Phase.scan
    @Published var deviceName = UserDefaults.standard.string(forKey: "pairing.deviceName") ?? "iPhone"

    private var task: Task<Void, Never>?
    /// Jeder Ablauf zählt hoch; Meldungen eines abgebrochenen werden verworfen.
    private var generation = 0
    private var decision: CheckedContinuation<Bool, Never>?
    private weak var sheet: UIViewController?

    #if DEBUG
    private let allowLocalHTTP = true  // lokaler Test-Hub im Simulator
    private let autoAccept = ProcessInfo.processInfo.arguments.contains("--pair-auto")
    #else
    private let allowLocalHTTP = false
    private let autoAccept = false
    #endif

    private var isBusy: Bool {
        switch phase {
        case .running, .confirmAccount: return true
        default: return false
        }
    }

    func startScan() {
        guard !isBusy else { return present() }
        phase = .scan
        present()
    }

    /// `localvoice://pair?…` aus der System-Kamera. Startet nie von selbst.
    func open(_ url: URL) {
        guard url.scheme?.lowercased() == "localvoice", url.host?.lowercased() == "pair", !isBusy else { return }
        accept(url.absoluteString, viaLink: true)
        present()
    }

    func scanned(_ text: String) {
        guard phase == .scan else { return }
        accept(text, viaLink: false)
    }

    private func accept(_ text: String, viaLink: Bool) {
        if let account = LibrarySync.shared.account {
            phase = .failed("Dieses iPhone ist schon als \(account.email) angemeldet. Zum Koppeln zuerst in „Konto & Sync“ abmelden.")
            return
        }
        do {
            let code = try PairingCode(uri: text, allowLocalHTTP: allowLocalHTTP)
            if viaLink || code.hub != LibrarySync.defaultHub {
                phase = .ready(code, viaLink: viaLink)
                if autoAccept { begin(code, viaLink: viaLink) }
            } else {
                begin(code, viaLink: false)
            }
        } catch {
            phase = .failed(error.localizedDescription)
        }
    }

    func begin(_ code: PairingCode, viaLink: Bool) {
        let name = PairingName.clean(deviceName)
        deviceName = name
        UserDefaults.standard.set(name, forKey: "pairing.deviceName")
        phase = .running(.claiming)
        generation += 1
        let run = generation
        // Ein Link oder fremder Server kann ein fremdes Konto bringen: vor dem Speichern fragen.
        let ask = viaLink || code.hub != LibrarySync.defaultHub
        let session = PairingSession(code: code, name: name, transport: HubTransport(base: code.hub))
        task = Task { [weak self] in
            var email = ""
            do {
                let confirmed = try await session.run(stage: { stage in
                    await MainActor.run { self?.show(stage, run) }
                }, adopt: { paired in
                    guard let self else { throw PairingError.declined }
                    try await self.adopt(paired, ask: ask, run: run)
                    await MainActor.run { email = paired.email }
                })
                if self?.generation == run { self?.phase = .done(email: email, confirmed: confirmed) }
            } catch is CancellationError {
                return
            } catch {
                if self?.generation == run { self?.phase = .failed(error.localizedDescription) }
            }
        }
    }

    private func show(_ stage: PairingStage, _ run: Int) {
        if generation == run { phase = .running(stage) }
    }

    private func adopt(_ paired: PairedAccount, ask: Bool, run: Int) async throws {
        guard generation == run else { throw CancellationError() }
        if ask && !autoAccept {
            phase = .confirmAccount(paired.email)
            let accepted = await withCheckedContinuation { decision = $0 }
            guard accepted, generation == run else { throw PairingError.declined }
            phase = .running(.opening)
        }
        try LibrarySync.shared.adopt(paired)
    }

    func decide(_ accepted: Bool) {
        decision?.resume(returning: accepted)
        decision = nil
    }

    func cancel() {
        generation += 1  // vor `decide`, damit die Antwort des alten Ablaufs nichts mehr anzeigt
        decide(false)
        task?.cancel()
        task = nil
        phase = .scan  // nimmt auch den Code samt `q` aus dem Speicher
    }

    func close() {
        cancel()
        sheet?.dismiss(animated: true)
    }

    /// Eigene Darstellung über dem obersten Blatt: Der Ablauf kommt aus „Konto & Sync“
    /// (selbst ein Blatt) oder von einem Link, während irgendein Blatt offen ist.
    private func present(attempt: Int = 0) {
        guard sheet?.presentingViewController == nil else { return }
        let window = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows).first(where: \.isKeyWindow)
        guard var top = window?.rootViewController else {
            // Kaltstart über den Link: Das Fenster kommt erst gleich.
            if attempt < 20 {
                Task { [weak self] in
                    try? await Task.sleep(nanoseconds: 150_000_000)
                    self?.present(attempt: attempt + 1)
                }
            }
            return
        }
        while let next = top.presentedViewController { top = next }
        let host = UIHostingController(rootView: PairingSheet())
        host.isModalInPresentation = true  // Schließen nur über den Knopf, damit der Ablauf sauber endet
        sheet = host
        top.present(host, animated: true)
    }
}

// MARK: Oberfläche

private struct PairingSheet: View {
    @ObservedObject private var flow = PairingFlow.shared
    @ScaledMetric(relativeTo: .largeTitle) private var sasSize: CGFloat = 52

    var body: some View {
        NavigationStack {
            content
                .navigationTitle("Mit Desktop-App verbinden").navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        if canCancel { Button("Abbrechen") { flow.close() } }
                    }
                }
        }
    }

    /// Beim Übernehmen nicht mehr: Das Konto kann schon gespeichert sein.
    private var canCancel: Bool {
        switch flow.phase {
        case .done, .running(.opening): return false
        default: return true
        }
    }

    @ViewBuilder private var content: some View {
        switch flow.phase {
        case .scan: PairingScanStep()
        case .ready(let code, let viaLink): readyStep(code, viaLink: viaLink)
        case .running(.compare(let sas)): compareStep(sas)
        case .running(.opening): progress("Konto wird übernommen …")
        case .running: progress("Verbinde mit dem PC …")
        case .confirmAccount(let email): accountStep(email)
        case .done(let email, let confirmed): doneStep(email, confirmed: confirmed)
        case .failed(let message): failedStep(message)
        }
    }

    private func progress(_ text: String) -> some View {
        VStack(spacing: 16) {
            ProgressView().controlSize(.large)
            Text(text).foregroundStyle(.secondary)
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func readyStep(_ code: PairingCode, viaLink: Bool) -> some View {
        Form {
            Section {
                LabeledContent("Server", value: code.hubHost)
                TextField("Gerätename", text: $flow.deviceName)
            } footer: {
                VStack(alignment: .leading, spacing: 6) {
                    if code.hub != LibrarySync.defaultHub { Text("Dieser Code nennt einen anderen Server als üblich. Nur fortfahren, wenn du ihm vertraust.") }
                    if viaLink { Text("Nur fortfahren, wenn du das Verbinden gerade selbst am eigenen PC gestartet hast.") }
                }
            }
            Section { Button("Verbinden") { flow.begin(code, viaLink: viaLink) } }
        }
    }

    private func compareStep(_ sas: String) -> some View {
        ScrollView {
            VStack(spacing: 20) {
                Text("Vergleichscode").font(.headline)
                Text(sas.prefix(3) + " " + sas.suffix(3))
                    .font(.system(size: sasSize, weight: .semibold, design: .monospaced))
                    .lineLimit(1).minimumScaleFactor(0.5)
                    .accessibilityLabel(sas.map(String.init).joined(separator: " "))
                Text("Steht derselbe Code am PC? Dann dort bestätigen. Wenn nicht, am PC abbrechen.")
                    .multilineTextAlignment(.center).foregroundStyle(.secondary)
                ProgressView("Bitte am PC bestätigen")
            }.padding(.horizontal, 24).padding(.vertical, 80).frame(maxWidth: .infinity)
        }
    }

    private func accountStep(_ email: String) -> some View {
        Form {
            Section {
                LabeledContent("Konto", value: email)
            } footer: {
                Text("Das iPhone gleicht deine Bibliothek künftig mit diesem Konto ab. Nur übernehmen, wenn es dein eigenes ist.")
            }
            Section {
                Button("Konto übernehmen") { flow.decide(true) }
                Button("Nicht übernehmen", role: .destructive) { flow.decide(false) }
            }
        }
    }

    private func doneStep(_ email: String, confirmed: Bool) -> some View {
        ScrollView {
            VStack(spacing: 16) {
                Image(systemName: "checkmark.circle.fill").font(.largeTitle).imageScale(.large).foregroundStyle(.green)
                Text("Verbunden").font(.title2.weight(.semibold))
                Text("Angemeldet als \(email). Das iPhone gleicht jetzt ab.")
                    .multilineTextAlignment(.center).foregroundStyle(.secondary)
                if !confirmed {
                    Text("Ob die Bestätigung am PC angekommen ist, ist unklar. Er zeigt vielleicht eine Warnung. Das iPhone bleibt angemeldet.")
                        .font(.footnote).multilineTextAlignment(.center).foregroundStyle(.secondary)
                }
                Button("Fertig") { flow.close() }.buttonStyle(.borderedProminent).padding(.top, 24)
            }.padding(.horizontal, 24).padding(.vertical, 80).frame(maxWidth: .infinity)
        }
    }

    private func failedStep(_ message: String) -> some View {
        ScrollView {
            VStack(spacing: 16) {
                Image(systemName: "exclamationmark.triangle.fill").font(.largeTitle).imageScale(.large).foregroundStyle(.orange)
                Text(message).multilineTextAlignment(.center)
                Text("Die Anmeldung mit Passwort bleibt in „Konto & Sync“ möglich.")
                    .font(.footnote).multilineTextAlignment(.center).foregroundStyle(.secondary)
                Button("Neuen Code scannen") { flow.startScan() }.buttonStyle(.borderedProminent).padding(.top, 24)
            }.padding(.horizontal, 24).padding(.vertical, 80).frame(maxWidth: .infinity)
        }
    }
}

private struct PairingScanStep: View {
    @ObservedObject private var flow = PairingFlow.shared
    @State private var access = AVCaptureDevice.authorizationStatus(for: .video)

    var body: some View {
        VStack(spacing: 12) {
            scanner
            Text("Am PC unter Einstellungen › Konto & Geräte auf „Handy verbinden“ gehen und den Code ins Bild halten.")
                .font(.footnote).multilineTextAlignment(.center).foregroundStyle(.secondary)
            TextField("Gerätename", text: $flow.deviceName).textFieldStyle(.roundedBorder)
        }
        .padding(16)
        .task {
            if access == .notDetermined {
                _ = await AVCaptureDevice.requestAccess(for: .video)
                access = AVCaptureDevice.authorizationStatus(for: .video)
            }
        }
    }

    @ViewBuilder private var scanner: some View {
        if access == .notDetermined {
            Spacer()
        } else if access != .authorized {
            ContentUnavailableView {
                Label("Kamera nicht freigegeben", systemImage: "camera.fill")
            } description: {
                Text("Zum Scannen die Kamera in den Einstellungen erlauben, oder in „Konto & Sync“ mit Passwort anmelden.")
            } actions: {
                if let url = URL(string: UIApplication.openSettingsURLString) { Link("Einstellungen öffnen", destination: url) }
            }
        } else if !DataScannerViewController.isSupported || !DataScannerViewController.isAvailable {
            ContentUnavailableView("Scanner nicht verfügbar", systemImage: "qrcode.viewfinder",
                                   description: Text("Auf diesem Gerät lässt sich gerade nicht scannen. Der Code kann auch mit der Kamera-App geöffnet werden."))
        } else {
            PairingScanner { flow.scanned($0) }.clipShape(RoundedRectangle(cornerRadius: 16))
        }
    }
}

private struct PairingScanner: UIViewControllerRepresentable {
    let found: (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(found: found) }

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(recognizedDataTypes: [.barcode(symbologies: [.qr])], qualityLevel: .balanced,
                                                recognizesMultipleItems: false, isHighFrameRateTrackingEnabled: false,
                                                isHighlightingEnabled: true)
        scanner.delegate = context.coordinator
        return scanner
    }

    func updateUIViewController(_ scanner: DataScannerViewController, context: Context) {
        if !scanner.isScanning { try? scanner.startScanning() }
    }

    static func dismantleUIViewController(_ scanner: DataScannerViewController, coordinator: Coordinator) {
        scanner.stopScanning()
    }

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let found: (String) -> Void
        init(found: @escaping (String) -> Void) { self.found = found }

        func dataScanner(_ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem], allItems: [RecognizedItem]) {
            for case .barcode(let code) in addedItems {
                // Fremde QR-Codes im Bild stören nicht, der Scanner läuft weiter.
                if let text = code.payloadStringValue, text.lowercased().hasPrefix("localvoice://pair") { found(text); return }
            }
        }
    }
}
