//! Small helper for the CLI side of third-party providers.
//!
//! `grok-providers sync` writes enabled entries from `providers.toml` into
//! `~/.grok/config.toml` using the upstream `[model_providers]` / `[model.*]`
//! schema. API keys are not copied; the CLI reads them from the environment
//! variables named in that file.

use grok_providers::{
    SecretStore, apply_choice, grok_config_path, load_or_default_store, providers_path,
    refresh_xai_models_blocking, save_provider_store, sync_cli_config,
};

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".into());
    let result = match command.as_str() {
        "sync" => sync(),
        "refresh-models" => refresh_models(),
        "path" => {
            println!("providers {}", providers_path().display());
            println!("config    {}", grok_config_path().display());
            Ok(())
        }
        "presets" => {
            println!("{}", include_str!("presets.example.toml"));
            Ok(())
        }
        _ => {
            eprintln!(
                "grok-providers — third-party model providers for the grok CLI\n\n\
                 Commands:\n  \
                 sync            merge enabled providers.toml entries into config.toml\n  \
                 refresh-models  fetch GET /v1/models and point Grok at the newest model\n  \
                 path            print providers.toml and config.toml paths\n  \
                 presets         print an example providers.toml\n\n\
                 Keys come from env vars (XAI_API_KEY, DEEPSEEK_API_KEY,\n\
                 MOONSHOT_API_KEY, OPENAI_API_KEY, ANTHROPIC_API_KEY), a grok login\n\
                 session, or the desktop app's keychain.\n\
                 XAI_MODEL pins a Grok model. Otherwise the newest general model\n\
                 from the cache (or grok-4.7 offline) is used.\n\
                 Set GROK_DISABLE_PROVIDER_PRESETS=1 to skip automatic env presets."
            );
            Ok(())
        }
    };
    if let Err(error) = result {
        eprintln!("grok-providers: {error}");
        std::process::exit(1);
    }
}

fn refresh_models() -> Result<(), String> {
    let mut store = load_or_default_store();
    let xai = store
        .providers
        .iter()
        .find(|provider| provider.id == "xai")
        .cloned()
        .ok_or_else(|| "xAI preset is missing from the provider list".to_string())?;
    let key = SecretStore::open().resolve(&xai, None)?;
    if key.is_none() {
        println!(
            "No xAI key or grok login session. Using the cached list, or grok-4.7 when there is no cache."
        );
    }
    let choice = refresh_xai_models_blocking(&xai.base_url, key.as_deref());
    apply_choice(&mut store, &choice);
    if let Ok(pinned) = std::env::var("XAI_MODEL") {
        let pinned = pinned.trim();
        if !pinned.is_empty() {
            if let Some(provider) = store
                .providers
                .iter_mut()
                .find(|provider| provider.id == "xai")
            {
                provider.model = pinned.to_string();
                provider.follow_latest = false;
            }
        }
    }
    save_provider_store(&store)?;
    let enabled: Vec<_> = store
        .providers
        .iter()
        .filter(|provider| provider.enabled)
        .cloned()
        .collect();
    sync_cli_config(&enabled)?;
    let model = store
        .providers
        .iter()
        .find(|provider| provider.id == "xai")
        .map(|provider| provider.model.as_str())
        .unwrap_or(choice.latest.as_str());
    println!("Grok model: {model}");
    println!("Hints: {}", choice.models.join(", "));
    Ok(())
}

fn sync() -> Result<(), String> {
    let store = load_or_default_store();
    let enabled: Vec<_> = store
        .providers
        .iter()
        .filter(|provider| provider.enabled)
        .cloned()
        .collect();
    if enabled.is_empty() {
        println!(
            "No enabled providers in {}. Enable one in the desktop settings or set enabled = true.",
            providers_path().display()
        );
        return Ok(());
    }
    let path = sync_cli_config(&enabled)?;
    println!("Updated {}", path.display());
    for provider in &enabled {
        println!(
            "  {id}: model {model} via {base} (key from ${env})",
            id = provider.id,
            model = provider.model,
            base = provider.base_url,
            env = provider.env_key
        );
    }
    Ok(())
}
