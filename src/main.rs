mod client;
mod commands;
mod config;
mod error;
mod progress;

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
  share download ABC123              Download by share code
  share info ABC123                  Check file info
  share login sa_your_token_here      Save personal token
  share list                         View upload history"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Upload files
    Upload {
        /// Files to upload
        files: Vec<PathBuf>,

        /// Password for download (requires personal token)
        #[arg(short, long)]
        password: Option<String>,

        /// Expiration: 5m, 30m, 1h, 3h, 6h, 12h, 24h (requires personal token)
        #[arg(short, long)]
        expires: Option<String>,

        /// One-time download (requires personal token)
        #[arg(long)]
        one_time: bool,

        /// File name for stdin upload
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Download a shared file
    Download {
        /// Share code
        code: String,

        /// Password if required
        #[arg(short, long)]
        password: Option<String>,

        /// Output path
        output: Option<PathBuf>,

        /// Specific file ID to download
        #[arg(long)]
        file_id: Option<String>,
    },

    /// Show file info before downloading
    Info {
        /// Share code
        code: String,
    },

    /// List your upload history (requires personal token)
    List,

    /// Save personal token for authenticated access
    Login {
        /// Personal token (starts with sa_). If omitted, opens browser sign-in
        token: Option<String>,
    },

    /// Remove saved personal token
    Logout,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let cfg = config::CliConfig::load();

    let result = match cli.command {
        Commands::Upload {
            files,
            password,
            expires,
            one_time,
            name,
        } => {
            let api_client = match client::ApiClient::new(&cfg) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\x1b[31mError: {}\x1b[0m", e);
                    std::process::exit(1);
                }
            };

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

            commands::upload::run(&api_client, files, stdin_data, name, password, expires, one_time).await
        }

        Commands::Download {
            code,
            password,
            output,
            file_id,
        } => {
            let api_client = match client::ApiClient::new(&cfg) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\x1b[31mError: {}\x1b[0m", e);
                    std::process::exit(1);
                }
            };
            commands::download::run(&api_client, code, password, output, file_id).await
        }

        Commands::Info { code } => {
            let api_client = match client::ApiClient::new(&cfg) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\x1b[31mError: {}\x1b[0m", e);
                    std::process::exit(1);
                }
            };
            commands::info::run(&api_client, code).await
        }

        Commands::List => {
            let api_client = match client::ApiClient::new(&cfg) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\x1b[31mError: {}\x1b[0m", e);
                    std::process::exit(1);
                }
            };
            commands::list::run(&api_client).await
        }

        Commands::Login { token } => commands::login::run(token, &cfg).await,

        Commands::Logout => commands::logout::run(),
    };

    if let Err(e) = result {
        eprintln!("\x1b[31mError: {}\x1b[0m", e);
        std::process::exit(1);
    }
}
