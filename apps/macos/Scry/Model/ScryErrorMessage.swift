import Foundation

extension ScryError {
    /// The core's sentence. The generated `localizedDescription` is the
    /// debug text, `Failed(message: …)`.
    var sentence: String {
        switch self {
        case .Failed(let message), .ChangedOnDisk(let message), .Refused(let message),
             .Git(let message), .GitMissing(let message), .NotARepository(let message):
            message
        }
    }
}

extension Error {
    /// What the app shows: the core's sentence for its own errors.
    var message: String {
        (self as? ScryError)?.sentence ?? localizedDescription
    }
}
