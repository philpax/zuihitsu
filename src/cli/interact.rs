//! The textual CLI for driving a participant turn — deliver a message with its senders and present
//! set, imprint the self-model, or note a participant arriving mid-conversation.

use clap::Subcommand;

use zuihitsu::{MessageInput, PersonId};

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
        } => {
            let sender_id = PersonId::new(platform, sender);
            let present_ids: Vec<PersonId> =
                present.iter().map(|u| PersonId::new(platform, u)).collect();
            print_json(&client.send(
                platform,
                scope,
                &[MessageInput {
                    sender: sender_id.clone(),
                    text: text.clone(),
                }],
                &present_ids,
            )?)
        }
        InteractCommand::Join {
            platform,
            scope,
            participant,
        } => {
            let participant_id = PersonId::new(platform, participant);
            client.join(platform, scope, &participant_id)?;
            tracing::info!(%platform, %scope, %participant, "noted join");
            Ok(())
        }
    }
}
