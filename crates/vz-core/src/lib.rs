//! Swappable pipeline boundaries. Implementations arrive after the Phase 0 scaffold.

/// A component failure with a human-readable reason.
pub type ComponentResult<T> = Result<T, String>;

/// Extract text from a document's bytes.
pub trait Parser {
    /// Parse one document without assuming a filesystem or network.
    fn parse(&self, document: &[u8]) -> ComponentResult<String>;
}

/// Extract text from an encoded page image.
pub trait Ocr {
    /// Recognize one image; unsupported platforms must return a reason.
    fn recognize(&self, image: &[u8]) -> ComponentResult<String>;
}

/// Convert text to a model-specific vector.
pub trait Embedder {
    /// Embed one text using the configured recipe.
    fn embed(&self, text: &str) -> ComponentResult<Vec<f32>>;
}

/// Answer a question using caller-supplied context.
pub trait ChatEngine {
    /// Produce an answer; streaming and citations are deferred to the pipeline spike.
    fn answer(&self, question: &str, context: &str) -> ComponentResult<String>;
}

/// Store and retrieve document text by stable identifier.
pub trait Store {
    /// Replace a document's text.
    fn put(&mut self, id: &str, text: &str) -> ComponentResult<()>;
    /// Read a document, returning None when absent.
    fn get(&self, id: &str) -> ComponentResult<Option<String>>;
}

/// Portable scaffold diagnostic; this does not probe future pipeline components.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SelfTestResult {
    /// "OK" when the scaffold can execute.
    pub status: &'static str,
    /// Core/app workspace version.
    pub version: &'static str,
    /// Operating system of this executable.
    pub os: &'static str,
    /// Architecture of this executable.
    pub arch: &'static str,
}

/// Return a deterministic, offline scaffold diagnostic.
pub fn self_test() -> SelfTestResult {
    SelfTestResult {
        status: "OK",
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_test_reports_this_build() {
        let result = self_test();
        assert_eq!(result.status, "OK");
        assert_eq!(result.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(result.os, std::env::consts::OS);
        assert_eq!(result.arch, std::env::consts::ARCH);
        assert!(!result.os.is_empty() && !result.arch.is_empty());
    }
}
