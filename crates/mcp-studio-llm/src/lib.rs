//! LLM provider abstraction (Anthropic first; OpenAI-compatible and Ollama later).

mod anthropic;
mod anthropic_messages;
mod sse;

pub use anthropic::{AnthropicCounter, DEFAULT_MODEL};
pub use anthropic_messages::AnthropicProvider;
pub use sse::{SseEvent, SseParser};

/// Address of the Anthropic API.
pub(crate) const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// Names of the supported providers.
pub const PROVIDERS: &[&str] = &["anthropic"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_is_supported() {
        assert!(PROVIDERS.contains(&"anthropic"));
    }
}
