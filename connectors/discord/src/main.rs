//! The zuihitsu Discord connector binary.
//!
//! A standalone bot that bridges Discord messages, presence, and joins into the zuihitsu platform
//! API. Connects to Discord with serenity and forwards messages to `POST /platform/message/stream`,
//! posting replies back to Discord.

use std::{path::PathBuf, sync::Arc};

use clap::Parser;
use serenity::{all::GatewayIntents, prelude::*};

mod addressing;
mod bot;
mod config;
mod context_sync;
mod error;
mod locator;
mod pacing;
mod turn_map;

use bot::{BotState, BotStateKey, Handler};
use config::DiscordConfig;

/// The zuihitsu Discord connector.
#[derive(Parser)]
struct Cli {
    /// Path to the connector config file.
    #[arg(long, default_value = "config.discord.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    let config = match DiscordConfig::load(&cli.config) {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(%error, "discord connector: fatal config error");
            std::process::exit(1);
        }
    };

    let token = config.discord.token.clone();
    let state = Arc::new(BotState::new(config));

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let mut client = Client::builder(&token, intents)
        .event_handler(Handler)
        .type_map_insert::<BotStateKey>(state.clone())
        .await
        .unwrap_or_else(|error| {
            tracing::error!(%error, "discord connector: fatal gateway error");
            std::process::exit(1);
        });

    // Shut down on Ctrl-C / SIGTERM.
    let shard_manager = client.shard_manager.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("could not install the Ctrl-C handler");
        tracing::info!("discord connector: shutting down");
        shard_manager.shutdown_all().await;
    });

    if let Err(error) = client.start().await {
        tracing::error!(%error, "discord connector: gateway error");
        std::process::exit(1);
    }
}
