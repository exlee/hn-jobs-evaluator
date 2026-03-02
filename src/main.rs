use serde::{Deserialize, Serialize};
use tokio;

mod comments;
mod evaluation;
mod gui;

#[derive(Serialize, Deserialize, Debug)]
struct RunSpec {
    pub hn_url: String,
    pub pdf_path: String,
    pub api_key: String,
    pub requirements: String,
}

#[tokio::main]
async fn main() -> eframe::Result {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    gui::main()
}
