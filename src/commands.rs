use crate::{Lookup, LookupInput, Slack};
use clap::{Parser, Subcommand};
use dekopon_provider_sdk::provider::{Proposal, Usage};

#[derive(Parser)]
#[command(
    name = "slack",
    about = "Narrow Slack operations; canvas content read is unsupported"
)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Canvas {
        #[command(subcommand)]
        operation: Canvas,
    },
}

#[derive(Subcommand)]
enum Canvas {
    Lookup {
        #[arg(value_name = "TEXT")]
        text: String,
    },
    Read,
}

pub(crate) fn propose(args: Args) -> Result<Proposal<Slack>, Usage> {
    match args.command {
        Command::Canvas {
            operation: Canvas::Lookup { text },
        } => Ok(Proposal::to::<Lookup>(LookupInput {
            contains_text: text,
        })),
        Command::Canvas {
            operation: Canvas::Read,
        } => Err(Usage::new(
            "canvas content read is unsupported; lookup returns section IDs only",
        )),
    }
}
