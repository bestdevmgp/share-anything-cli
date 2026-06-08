mod client;
mod commands;
mod config;
mod core;
mod error;
pub mod format;
mod p2p;
mod progress;
mod tui;
pub mod time;
mod update_check;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "share",
    version,
    about = "Share Anything CLI - Fast file sharing from the terminal",
    override_usage = "share <COMMAND>",
    before_help = "\x1b[1mShare Anything CLI\x1b[0m - Fast file sharing from the terminal\n  \x1b[2mhttps://share.mingyu.dev\x1b[0m",
    after_help = "\x1b[1mExamples:\x1b[0m
  share upload file.txt              Upload a file
  share upload a.txt b.txt           Upload multiple files
  echo 'hi' | share upload -n hi.txt Pipe stdin
  share download 123456              Download by share code
  share info 123456                  Check file info
  share login sat_your_token_here     Save personal token
  share history                      View upload history
  share download-history             View download history
  share delete 123456                Delete a share by code
  share logs 123456                  View download logs for a share"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Upload {
        files: Vec<PathBuf>,

        #[arg(short, long)]
        password: Option<String>,

        #[arg(short, long)]
        expires: Option<String>,

        #[arg(long)]
        one_time: bool,

        #[arg(short, long)]
        name: Option<String>,

        #[arg(short, long)]
        secure: bool,
    },

    Download {
        code: String,

        #[arg(short, long)]
        password: Option<String>,

        output: Option<PathBuf>,

        #[arg(long)]
        file_id: Option<String>,

        #[arg(long)]
        zip: bool,
    },

    Info {
        code: String,
    },

    #[command(alias = "list")]
    History,

    #[command(name = "download-history")]
    DownloadHistory,

    #[command(alias = "remove")]
    Delete {
        code: String,
    },

    #[command(alias = "log")]
    Logs {
        code: String,
    },

    Login {
        token: Option<String>,
    },

    Logout,
}

async fn run_cli(cli: Cli) -> Result<(), crate::error::CliError> {
    let cfg = config::CliConfig::load();

    match cli.command {
        Commands::Upload {
            files,
            password,
            expires,
            one_time,
            name,
            secure,
        } => {
            let api_client = client::ApiClient::new(&cfg)?;

            let stdin_data = if files.is_empty() && atty::isnt(atty::Stream::Stdin) {
                use std::io::Read;
                let mut buf = Vec::new();
                std::io::stdin().read_to_end(&mut buf).ok();
                if buf.is_empty() {
                    None
                } else {
                    Some(buf)
                }
            } else {
                None
            };

            if files.is_empty() && stdin_data.is_none() {
                eprintln!("\x1b[31mError: No files specified. Provide file paths or pipe data via stdin.\x1b[0m");
                eprintln!("  Usage: share upload <file1> [file2 ...]");
                eprintln!("  Pipe:  echo 'hello' | share upload --name hello.txt");
                std::process::exit(1);
            }

            if secure {
                commands::upload::run_secure(&api_client, files, stdin_data, name, password).await
            } else {
                commands::upload::run(&api_client, files, stdin_data, name, password, expires, one_time).await
            }
        }

        Commands::Download {
            code,
            password,
            output,
            file_id,
            zip,
        } => {
            let api_client = client::ApiClient::new(&cfg)?;
            commands::download::run(&api_client, code, password, output, file_id, zip).await
        }

        Commands::Info { code } => {
            let api_client = client::ApiClient::new(&cfg)?;
            commands::info::run(&api_client, code).await
        }

        Commands::History => {
            let api_client = client::ApiClient::new(&cfg)?;
            commands::list::run(&api_client).await
        }

        Commands::DownloadHistory => {
            let api_client = client::ApiClient::new(&cfg)?;
            commands::download_history::run(&api_client).await
        }

        Commands::Delete { code } => {
            let api_client = client::ApiClient::new(&cfg)?;
            commands::delete::run(&api_client, code).await
        }

        Commands::Logs { code } => {
            let api_client = client::ApiClient::new(&cfg)?;
            commands::logs::run(&api_client, code).await
        }

        Commands::Login { token } => commands::login::run(token, &cfg).await,

        Commands::Logout => commands::logout::run(),
    }
}

#[tokio::main]
async fn main() {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    if raw_args.is_empty() {
        let stdin_tty = atty::is(atty::Stream::Stdin);
        let stdout_tty = atty::is(atty::Stream::Stdout);

        if stdin_tty && stdout_tty {
            let cfg = config::CliConfig::load();
            if let Err(e) = tui::run(cfg).await {
                eprintln!("\x1b[31mError: {}\x1b[0m", e);
                std::process::exit(1);
            }
            return;
        }

        if !stdin_tty {
            let cli = Cli::parse_from(["share", "upload"]);
            if let Err(e) = run_cli(cli).await {
                eprintln!("\x1b[31mError: {}\x1b[0m", e);
                std::process::exit(1);
            }
            return;
        }

        eprintln!("share: cannot launch TUI (stdout is not a TTY).");
        eprintln!("Run `share --help` for available commands.");
        std::process::exit(1);
    }

    let cli = Cli::parse();
    if let Err(e) = run_cli(cli).await {
        eprintln!("\x1b[31mError: {}\x1b[0m", e);
        std::process::exit(1);
    }
}
