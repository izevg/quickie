//! The local LLM servers Quickie knows how to find on this machine.

/// A local LLM server on its default loopback port. Quickie probes for them only when the user
/// asks it to look, never on its own: probing is a connection like any other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LocalServer {
    /// Ollama, on port 11434.
    Ollama,
    /// LM Studio's server, on port 1234.
    LmStudio,
}

impl LocalServer {
    /// Every server Quickie looks for, in the order it shows them.
    pub const ALL: [Self; 2] = [Self::Ollama, Self::LmStudio];

    /// The server's name, as its makers spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LmStudio => "LM Studio",
        }
    }

    /// The base URL of the server's OpenAI-compatible API.
    #[must_use]
    pub const fn base_url(self) -> &'static str {
        match self {
            Self::Ollama => "http://127.0.0.1:11434/v1/",
            Self::LmStudio => "http://127.0.0.1:1234/v1/",
        }
    }

    /// Where the server lists its models: Ollama's own `/api/tags`, LM Studio's `/v1/models`.
    #[must_use]
    pub const fn probe_url(self) -> &'static str {
        match self {
            Self::Ollama => "http://127.0.0.1:11434/api/tags",
            Self::LmStudio => "http://127.0.0.1:1234/v1/models",
        }
    }
}
