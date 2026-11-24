import Foundation
import Testing
@testable import Scry

/// Explain's asks and answers in memory, as the agent would leave them in the
/// repository: these tests never ask a real Claude Code.
@MainActor
private final class FakeExplainer: Explainer {
    var askedAt: [Int: UInt64] = [:]
    var answers: [Int: Explanation] = [:]
    var watching = true
    var available = true
    var stale = false
    var failure: ScryError?
    private(set) var asks: [Int] = []

    func explainState(_ index: Int) -> ExplainState {
        ExplainState(available: available, askedAt: askedAt[index], answer: answers[index], watching: watching, stale: stale)
    }

    func askExplain(_ index: Int) throws {
        if let failure { throw failure }
        asks.append(index)
        askedAt[index] = UInt64(Date().timeIntervalSince1970 * 1000)
    }

    func cancelExplain(_ index: Int) throws {
        askedAt[index] = nil
    }

    func answer(_ index: Int, _ text: String, failed: Bool = false) {
        askedAt[index] = nil
        answers[index] = Explanation(
            text: text, by: "Claude Code", at: UInt64(Date().timeIntervalSince1970 * 1000), failed: failed
        )
    }
}

@MainActor
struct ExplainTests {
    private func inspector(_ explainer: FakeExplainer, defaults: UserDefaults = .isolated) throws -> InspectorViewModel {
        let model = try MergeModel(path: ConflictFile().url.path)
        return InspectorViewModel(model: model, explainer: explainer, defaults: defaults)
    }

    /// Asked with a session watching, the block waits and counts; the agent's answer
    /// then shows with who wrote it. Asked again, the answer stays below the new wait.
    @Test func anAskWaitsThenShowsTheAnswer() throws {
        let explainer = FakeExplainer()
        let inspector = try inspector(explainer)
        #expect(inspector.explain == nil)
        inspector.refreshExplain()
        #expect(inspector.explain == .notAsked(watching: true))
        #expect(inspector.canExplain)

        inspector.askExplain()
        #expect(explainer.asks == [0])
        guard case .waiting(let seconds, nil) = inspector.explain else {
            Issue.record("not waiting: \(String(describing: inspector.explain))")
            return
        }
        #expect(seconds < 5)

        explainer.answer(0, "Main raised the timeout; the feature renamed it.")
        inspector.refreshExplain()
        guard case .answered(let explanation, watching: true) = inspector.explain else {
            Issue.record("not answered: \(String(describing: inspector.explain))")
            return
        }
        #expect(explanation.by == "Claude Code")

        inspector.askExplain()
        guard case .waiting(_, let previous) = inspector.explain else {
            Issue.record("not waiting again")
            return
        }
        #expect(previous == explanation)
        inspector.cancelExplain()
        #expect(inspector.explain == .answered(explanation, watching: true))
    }

    /// Nobody watching, the ask is queued and the block says to open Claude Code;
    /// a session watching for a minute without an answer is "no answer".
    @Test func noSessionAndNoAnswer() throws {
        let explainer = FakeExplainer()
        explainer.watching = false
        let inspector = try inspector(explainer)
        inspector.refreshExplain()
        #expect(inspector.explain == .notAsked(watching: false))
        #expect(inspector.watching == false)
        inspector.askExplain()
        #expect(inspector.explain == .noSession(previous: nil))

        explainer.watching = true
        explainer.askedAt[0] = UInt64((Date().timeIntervalSince1970 - 61) * 1000)
        inspector.refreshExplain()
        #expect(inspector.watching == true)
        #expect(inspector.explain == .noAnswer(previous: nil))
        #expect(inspector.canExplain)
    }

    /// Off in Settings › Agent, or for a file in no repository, there is no Explain:
    /// no block, no ⌃⌘E, nothing in the ⌘/ sheet. Whether Claude Code is here still
    /// shows: the agent can resolve. Turned on again, the answers come back.
    @Test func offOrOutsideARepositoryThereIsNoExplain() throws {
        let explainer = FakeExplainer()
        explainer.answer(0, "An answer kept while Explain was off.")
        let defaults = UserDefaults.isolated
        defaults.set(false, forKey: InspectorViewModel.explainKey)
        let inspector = try inspector(explainer, defaults: defaults)
        inspector.refreshExplain()
        #expect(inspector.explain == nil && inspector.watching == true)
        let sheet = KeyboardShortcutsViewModel(current: .editor, defaults: defaults)
        #expect(!sheet.groups.flatMap(\.rows).contains { $0.id == Shortcut.explain.id })
        #expect(KeyboardShortcutsViewModel(current: .editor, defaults: .isolated)
            .groups.flatMap(\.rows).contains { $0.id == Shortcut.explain.id })
        #expect(!inspector.explainOn && !inspector.canExplain)
        inspector.askExplain()
        #expect(explainer.asks.isEmpty)

        defaults.set(true, forKey: InspectorViewModel.explainKey)
        inspector.refreshExplain()
        guard case .answered = inspector.explain else {
            Issue.record("answer not back")
            return
        }

        explainer.available = false
        inspector.refreshExplain()
        #expect(inspector.explain == nil && inspector.watching == nil)
    }

    /// The indicator shows until Settings › Agent finds Scry's plugin off, and
    /// again once it finds it on.
    @Test func theIndicatorFollowsThePlugin() throws {
        let defaults = UserDefaults.isolated
        let inspector = try inspector(FakeExplainer(), defaults: defaults)
        inspector.refreshExplain()
        #expect(inspector.watching == true)
        ClaudeCodePlugin.remember(.disabled, in: defaults)
        #expect(inspector.watching == nil)
        #expect(inspector.explain == .notAsked(watching: true))
        ClaudeCodePlugin.remember(.installed, in: defaults)
        #expect(inspector.watching == true)
        ClaudeCodePlugin.remember(.noClaude, in: defaults)
        #expect(inspector.watching == true)
    }

    /// An answer about text rewritten since is stale, and can be asked again; a
    /// failure shows the agent's problem and is not kept below a new ask.
    @Test func staleAndFailedAnswers() throws {
        let explainer = FakeExplainer()
        let inspector = try inspector(explainer)
        explainer.answer(0, "About the text written by hand then.")
        explainer.stale = true
        inspector.refreshExplain()
        guard case .stale(let explanation) = inspector.explain else {
            Issue.record("not stale: \(String(describing: inspector.explain))")
            return
        }
        #expect(inspector.canExplain)
        inspector.askExplain()
        #expect(inspector.explain == .waiting(seconds: 0, previous: explanation))

        explainer.answer(0, "conflict found no conflict c7a9.", failed: true)
        inspector.refreshExplain()
        guard case .failed(let failure) = inspector.explain else {
            Issue.record("not failed: \(String(describing: inspector.explain))")
            return
        }
        #expect(failure.text == "conflict found no conflict c7a9.")
        inspector.askExplain()
        #expect(inspector.explain == .waiting(seconds: 0, previous: nil))
        explainer.watching = false
        inspector.refreshExplain()
        #expect(inspector.explain == .noSession(previous: nil))
    }

    /// A failing ask says why, and the next one clears it; a long answer opens and
    /// closes per conflict.
    @Test func failuresAndLongAnswers() throws {
        let explainer = FakeExplainer()
        explainer.failure = .Failed(message: "The ask could not be kept: disk full")
        let inspector = try inspector(explainer)
        inspector.refreshExplain()
        inspector.askExplain()
        #expect(inspector.explainError == "The ask could not be kept: disk full")
        explainer.failure = nil
        inspector.askExplain()
        #expect(inspector.explainError == nil)

        #expect(!inspector.isExpanded)
        inspector.toggleExpanded()
        #expect(inspector.isExpanded)
        #expect(ExplainBlock.isLong(String(repeating: "x", count: 500)))
        #expect(!ExplainBlock.isLong("Short."))
    }

    /// The Explain switch is kept per user, on by default.
    @Test func theSwitchIsKept() {
        let defaults = UserDefaults.isolated
        #expect(InspectorViewModel.explainEnabled(in: defaults))
        let settings = AgentSettingsViewModel(registrar: NoRegistrar(), defaults: defaults)
        #expect(settings.explainEnabled)
        settings.explainEnabled = false
        #expect(!InspectorViewModel.explainEnabled(in: defaults))
        #expect(settings.explainDetail.hasPrefix("Off:"))
    }
}

/// Settings › Agent without Claude Code, for the switch alone.
@MainActor
private struct NoRegistrar: AgentRegistrar {
    func setup() async -> AgentSetup {
        AgentSetup(state: .noClaude, command: "", oldServer: false, pluginScry: nil, toolsAllowed: false)
    }
    func install() async throws {}
    func uninstall() async throws {}
    func removeOldServer() async throws {}
    func allowTools() async throws {}
}
