// Derived from openrig packages/cli/src/provider-auth.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! How the provider selected in Codex `config.toml` authenticates.

use toml_edit::{DocumentMut, Item};

/// Codex auth inferred from `config.toml`. Unresolved cases keep the login check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexProviderAuth {
    /// `codex login status` is the check.
    OpenAiLogin {
        /// Why a configured provider was not used, when that happened.
        unresolved: Option<String>,
    },
    /// The provider reads a credential from the environment.
    EnvKey {
        /// `model_provider` value.
        provider_id: String,
        /// Environment variable name.
        env_key: String,
    },
}

/// Read `model_provider` / `model_providers` the way the TypeScript helper does.
#[must_use]
pub fn select_codex_provider_auth(config_toml: Option<&str>) -> CodexProviderAuth {
    let Some(text) = config_toml else {
        return CodexProviderAuth::OpenAiLogin { unresolved: None };
    };
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return CodexProviderAuth::OpenAiLogin {
            unresolved: Some("config.toml could not be parsed".to_owned()),
        };
    };
    if doc.contains_key("profile") {
        return CodexProviderAuth::OpenAiLogin {
            unresolved: Some(
                "config.toml selects a legacy profile, which is not resolved here".to_owned(),
            ),
        };
    }
    let Some(provider_id) = doc.get("model_provider").and_then(Item::as_str) else {
        return CodexProviderAuth::OpenAiLogin { unresolved: None };
    };
    let Some(providers) = doc.get("model_providers").and_then(Item::as_table) else {
        return CodexProviderAuth::OpenAiLogin { unresolved: None };
    };
    if !providers.contains_key(provider_id) {
        return CodexProviderAuth::OpenAiLogin { unresolved: None };
    }
    let Some(entry) = providers.get(provider_id).and_then(Item::as_table) else {
        return CodexProviderAuth::OpenAiLogin { unresolved: None };
    };
    let requires = entry.get("requires_openai_auth").and_then(Item::as_bool);
    let env_key = entry.get("env_key").and_then(Item::as_str);
    match (requires, env_key) {
        (Some(false), Some(key)) if is_env_key(key) => CodexProviderAuth::EnvKey {
            provider_id: provider_id.to_owned(),
            env_key: key.to_owned(),
        },
        _ => CodexProviderAuth::OpenAiLogin { unresolved: None },
    }
}

fn is_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}
