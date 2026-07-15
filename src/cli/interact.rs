//! The `interact` namespace: drive the agent — send an imprint interview message, deliver a
//! participant turn, or note a participant arriving mid-session.

use clap::Subcommand;

use zuihitsu::MessageInput;

use crate::cli::{client::Client, error::CliError, print_json};

#[derive(Subcommand)]
pub(crate) enum InteractCommand {
    /// Send one operator message of the imprint interview (prints the agent's reply).
    Imprint {
        #[arg(long)]
        text: String,
    },
    /// Deliver a participant message and print the agent's reply.
    Send {
        #[arg(long)]
        platform: String,
        #[arg(long)]
        scope: String,
        #[arg(long)]
        sender: String,
        #[arg(long)]
        text: String,
        /// A present participant; repeatable. The sender is always treated as present.
        #[arg(long = "present")]
        present: Vec<String>,
    },
    /// Note a participant arriving mid-session.
    Join {
        #[arg(long)]
        platform: String,
        #[arg(long)]
        scope: String,
        #[arg(long)]
        participant: String,
    },
}

pub(crate) fn dispatch(client: &Client, command: &InteractCommand) -> Result<(), CliError> {
    match command {
        InteractCommand::Imprint { text } => print_json(&client.imprint(text)?),
        InteractCommand::Send {
            platform,
            scope,
            sender,
            text,
            present,
        } => print_json(&client.send(
            platform,
            scope,
            &[MessageInput {
                sender: sender.clone(),
                text: text.clone(),
            }],
            present,
        )?),
        InteractCommand::Join {
            platform,
            scope,
            participant,
        } => {
            client.join(platform, scope, participant)?;
            tracing::info!(%platform, %scope, %participant, "noted join");
            Ok(())
        }
    }
}
