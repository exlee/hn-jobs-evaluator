use serde::{Deserialize, Serialize};
use tokio;

mod autofetcher;
mod comments;
mod common_gui;
mod evaluation;
mod gui;

#[tokio::main]
async fn main() -> eframe::Result {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    gui::main()
}
