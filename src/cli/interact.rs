//! The textual CLI for driving a participant turn — deliver a message with its senders and present
//! set, imprint the self-model, or note a participant arriving mid-conversation.

use clap::Subcommand;

use crate::cli::{client::Client, error::CliError, print_json};

#[derive(Subcommand)]
pub(crate) enum InteractCommand {
    /// Send one operator message of the imprint interview (prints the agent's reply).
    Imprint {
        #[arg(long)]
        text: String,
    },
    /// Deliver a participant message and print the agent's reply. The CLI is the operator's own
    /// loopback client, so the message is delivered under the `direct` interface — ids are bare, with
    /// no platform to name.
    Send {
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
    /// Note a participant arriving mid-session, under the `direct` interface.
    Join {
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
            scope,
            sender,
            text,
            present,
        } => print_json(&client.send(scope, sender, text, present)?),
        InteractCommand::Join { scope, participant } => {
            client.join(scope, participant)?;
            tracing::info!(%scope, %participant, "noted join");
            Ok(())
        }
    }
}
