use crate::{Lookup, LookupInput, Post, PostInput, Read, ReadInput, Replace, ReplaceInput, Slack};
use clap::{Parser, Subcommand};
use dekopon_provider_sdk::provider::{Proposal, Usage};

#[derive(Parser)]
#[command(
    name = "slack",
    about = "Narrow Slack operations on the owner-pinned canvas and channel messages"
)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Post {
        #[arg(value_name = "CHANNEL")]
        channel: String,
        #[arg(value_name = "TEXT")]
        text: String,
        #[arg(value_name = "THREAD_TS")]
        thread_ts: Option<String>,
    },
    Canvas {
        #[command(subcommand)]
        operation: Canvas,
    },
}

#[derive(Subcommand)]
enum Canvas {
    #[command(
        about = "List the IDs of every canvas section containing TEXT; the IDs are for information and are not replace input"
    )]
    Lookup {
        #[arg(value_name = "TEXT")]
        text: String,
    },
    #[command(
        about = "Replace the one canvas block that contains SECTION_MATCH with one block of Markdown",
        after_help = "After any failure, run `slack canvas read` before another edit."
    )]
    Replace {
        #[arg(
            value_name = "SECTION_MATCH",
            help = "Text that must appear in exactly one block of the canvas; it is search text, not a section ID"
        )]
        section_match: String,
        #[arg(
            value_name = "MARKDOWN",
            help = "One line of Markdown that replaces only the matched block; carry only that block's new text, never its surrounding heading or neighbouring blocks"
        )]
        markdown: String,
    },
    #[command(
        about = "Read the whole owner-pinned canvas as Markdown; edit a section with replace"
    )]
    Read,
}

pub(crate) fn propose(args: Args) -> Result<Proposal<Slack>, Usage> {
    match args.command {
        Command::Post {
            channel,
            text,
            thread_ts,
        } => Ok(Proposal::to::<Post>(PostInput {
            channel,
            text,
            thread_ts,
        })),
        Command::Canvas {
            operation: Canvas::Lookup { text },
        } => Ok(Proposal::to::<Lookup>(LookupInput {
            contains_text: text,
        })),
        Command::Canvas {
            operation:
                Canvas::Replace {
                    section_match,
                    markdown,
                },
        } => Ok(Proposal::to::<Replace>(ReplaceInput {
            section_match,
            markdown,
        })),
        Command::Canvas {
            operation: Canvas::Read,
        } => Ok(Proposal::to::<Read>(ReadInput {})),
    }
}
