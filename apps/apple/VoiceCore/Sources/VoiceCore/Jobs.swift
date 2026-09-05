import Foundation

public enum JobPhase: String, Codable, Sendable { case waiting, transcribing, generating, paused, cancelled, failed, completed }
public enum JobFailure: String, Codable, Sendable { case unavailable, noSpeech, invalidOutput, interrupted, storage }
public struct JobRecord: Codable, Sendable {
    public var phase: JobPhase = .waiting
    public var attempts = 0
    public var nextAttemptAt: Date?
    public var failure: JobFailure?
    public var running: Bool { phase == .transcribing || phase == .generating }
    public func canStart(now: Date) -> Bool {
        !running && phase != .completed && phase != .failed && phase != .cancelled && attempts < 3 && (nextAttemptAt.map { $0 <= now } ?? true)
    }
}
extension DurableStore {
    public func beginJob(_ id: UUID, now: Date = Date()) throws -> Bool {
        var entry = try loadEntry(for: id)
        guard entry.reply == nil else { return false }
        var job = entry.job ?? JobRecord()
        guard job.canStart(now: now) else { return false }
        job.attempts += 1; job.failure = nil; job.nextAttemptAt = nil
        job.phase = entry.transcript == nil ? .transcribing : .generating
        entry.job = job; try save(entry)
        return true
    }
    public func setJobPhase(_ id: UUID, phase: JobPhase) throws {
        var entry = try loadEntry(for: id)
        guard entry.reply == nil, var job = entry.job, job.running,
              phase == .generating else { throw VoiceError.invalid }
        job.phase = phase; entry.job = job; try save(entry)
    }
    public func failJob(_ id: UUID, reason: JobFailure, now: Date = Date()) throws {
        var entry = try loadEntry(for: id)
        guard entry.reply == nil, var job = entry.job else { return }
        job.failure = reason
        job.phase = job.attempts >= 3 || reason == .noSpeech || reason == .invalidOutput ? .failed : .waiting
        job.nextAttemptAt = job.phase == .waiting ? now.addingTimeInterval(5 * pow(2, Double(job.attempts - 1))) : nil
        entry.job = job; entry.state = .deferred; try save(entry)
    }
    public func pauseJob(_ id: UUID, userInitiated: Bool = false) throws {
        var entry = try loadEntry(for: id)
        guard entry.reply == nil, var job = entry.job, job.running else { return }
        job.phase = userInitiated ? .cancelled : .paused
        job.failure = .interrupted; job.nextAttemptAt = nil
        entry.job = job; entry.state = .deferred; try save(entry)
    }
    public func recoverInterruptedJobs() throws {
        for entry in try entries() where entry.job?.running == true { try pauseJob(entry.id) }
    }
    /// Explicit user retry is the only operation which replenishes a failed job's attempt budget.
    public func retryJob(_ id: UUID) throws {
        var entry = try loadEntry(for: id)
        guard entry.reply == nil, entry.job?.running != true else { return }
        entry.job = JobRecord(); entry.state = .deferred; try save(entry)
    }
}
