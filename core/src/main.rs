use serde::{Deserialize, Serialize};
use tokio;

mod comments;
mod evaluation;
mod gui;
mod common_gui;


#[tokio::main]
async fn main() -> eframe::Result {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    gui::main()
}
