// Derived from openrig packages/cli/src/provider-auth.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Codex provider auth selection from `config.toml`.

use crewmux_provider::{CodexProviderAuth, select_codex_provider_auth};

#[test]
fn env_key_skips_openai_login() {
    let text = r#"
model_provider = "proxy"
[model_providers.proxy]
requires_openai_auth = false
env_key = "PROXY_TOKEN"
"#;
    let CodexProviderAuth::EnvKey {
        provider_id,
        env_key,
    } = select_codex_provider_auth(Some(text))
    else {
        panic!("expected an env-key provider");
    };
    assert_eq!(provider_id, "proxy");
    assert_eq!(env_key, "PROXY_TOKEN");
}

#[test]
fn legacy_profile_and_bad_toml_keep_openai_login() {
    let profile = select_codex_provider_auth(Some("profile = \"work\"\n"));
    assert!(matches!(
        profile,
        CodexProviderAuth::OpenAiLogin {
            unresolved: Some(_)
        }
    ));
    let broken = select_codex_provider_auth(Some("this = [\n"));
    assert!(matches!(
        broken,
        CodexProviderAuth::OpenAiLogin {
            unresolved: Some(_)
        }
    ));
    assert!(matches!(
        select_codex_provider_auth(None),
        CodexProviderAuth::OpenAiLogin { unresolved: None }
    ));
}
