// Cargo.toml
// reqwest = { version = "0.12", features = ["json"] }
// serde = { version = "1", features = ["derive"] }
// tokio = { version = "1", features = ["full"] }

use reqwest::Client;
use serde::{Deserialize, Serialize};

const BASE: &str = "https://hacker-news.firebaseio.com/v0";
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Story {
    pub id: u64,
    pub title: Option<String>,
    pub url: Option<String>,
    pub score: Option<u32>,
    pub by: Option<String>,
    pub descendants: Option<u32>,
}

pub async fn get_front_page_stories() -> anyhow::Result<Vec<Story>> {
    let client = Client::new();
    // top 30 = front page
    let ids: Vec<u64> = client
        .get(format!("{BASE}/topstories.json"))
        .send()
        .await?
        .json()
        .await?;

    let stories = futures::future::join_all(ids.iter().take(30).map(|id| {
        let c = client.clone();
        async move {
            c.get(format!("{BASE}/item/{id}.json"))
                .send()
                .await?
                .json::<Story>()
                .await
        }
    }))
    .await;

    Ok(stories.into_iter().flatten().collect())
}

use std::{sync::Arc, time::Duration};

use tokio::{sync::mpsc::Sender, task::AbortHandle};
use tracing::Instrument as _;

use crate::{
    backend::app_service::{AppService, AppServiceDefault},
    demo::AppServiceDemo,
    events::{Event, EventEnvelope},
};
#[derive(Debug, Clone)]
pub struct FrontPageProcessor {
    pub app_service: Arc<dyn AppService>,
    pub handle: Option<AbortHandle>,
}

impl Default for FrontPageProcessor {
    fn default() -> Self {
        let app_service: Arc<dyn AppService> = if crate::demo::is_demo() {
            Arc::new(AppServiceDemo {})
        } else {
            Arc::new(AppServiceDefault)
        };
        Self {
            app_service,
            handle: None,
        }
    }
}
impl FrontPageProcessor {
    pub fn enable(&mut self, tx: Sender<EventEnvelope>) {
        if self.handle.is_some() {
            return;
        }
        let app_service = Arc::clone(&self.app_service);
        let join = tokio::task::spawn(async move {
            loop {
                let stories = app_service
                    .get_front_page_stories()
                    .instrument(tracing::info_span!("front_page_update_loop"))
                    .await;
                let _ = tx
                    .send(EventEnvelope {
                        event: Event::FrontPageUpdate { stories },
                        span: tracing::info_span!("front_page_processor"),
                    })
                    .await;
                let _ = tokio::time::sleep(Duration::from_secs(300)).await;
            }
        });
        self.handle = Some(join.abort_handle());
    }
    pub fn disable(&mut self) {
        if let Some(abort_handle) = self.handle.take() {
            abort_handle.abort();
        }
    }
}
