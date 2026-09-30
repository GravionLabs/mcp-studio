//! LLM provider abstraction (Anthropic first; OpenAI-compatible and Ollama later).

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
