//! Small helper for the CLI side of third-party providers.
//!
//! `grok-providers sync` writes enabled entries from `providers.toml` into
//! `~/.grok/config.toml` using the upstream `[model_providers]` / `[model.*]`
//! schema. API keys are not copied; the CLI reads them from the environment
//! variables named in that file.

use grok_providers::{grok_config_path, load_or_default_store, providers_path, sync_cli_config};

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".into());
    let result = match command.as_str() {
        "sync" => sync(),
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
                 sync      merge enabled providers.toml entries into config.toml\n  \
                 path      print providers.toml and config.toml paths\n  \
                 presets   print an example providers.toml\n\n\
                 Keys come from env vars (DEEPSEEK_API_KEY, MOONSHOT_API_KEY,\n\
                 OPENAI_API_KEY, ANTHROPIC_API_KEY) or the desktop app's keychain.\n\
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
