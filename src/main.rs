mod client;
mod commands;
mod config;
mod error;
mod progress;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "sa", version, about = "Share Anything CLI - fast file sharing from the terminal")]
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

        /// Password for download (requires API key)
        #[arg(short, long)]
        password: Option<String>,

        /// Expiration: 30m, 1h, 3h, 6h, 12h, 24h (requires API key)
        #[arg(short, long)]
        expires: Option<String>,

        /// One-time download (requires API key)
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
        #[arg(short, long)]
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

    /// List your upload history (requires API key)
    List,

    /// Save API key for authenticated access
    Login {
        /// API key (starts with sa_)
        api_key: String,
    },

    /// Remove saved API key
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

            // Check for stdin pipe
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
                eprintln!("  Usage: sa upload <file1> [file2 ...]");
                eprintln!("  Pipe:  echo 'hello' | sa upload --name hello.txt");
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

        Commands::Login { api_key } => commands::login::run(api_key),

        Commands::Logout => commands::logout::run(),
    };

    if let Err(e) = result {
        eprintln!("\x1b[31mError: {}\x1b[0m", e);
        std::process::exit(1);
    }
}
