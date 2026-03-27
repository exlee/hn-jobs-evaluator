use serde::{Deserialize, Serialize};
use tokio;
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};
use tracing_tree::HierarchicalLayer;

mod appstate_evaluation;
mod autofetcher;
mod comments;
mod common_gui;
mod evaluation;
mod gui;
mod job_description;
mod notify;

#[tokio::main]
async fn main() -> eframe::Result {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(HierarchicalLayer::new(2))
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    gui::main()
}
