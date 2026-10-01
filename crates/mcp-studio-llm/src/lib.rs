//! LLM provider abstraction (Anthropic first; OpenAI-compatible and Ollama later).

mod anthropic;
mod anthropic_messages;
mod openai_compat;
mod registry;
mod sse;

pub use anthropic::{AnthropicCounter, DEFAULT_MODEL};
pub use anthropic_messages::AnthropicProvider;
pub use openai_compat::OpenAiCompatProvider;
pub use registry::{
    load_settings, normalize_settings, ollama_models, resolve, save_settings, split_model,
    ProviderKind, ResolvedModel,
};
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
