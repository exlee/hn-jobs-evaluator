use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::{sync::mpsc::Sender, task::AbortHandle};
use tracing::Instrument as _;

use crate::{
    comments::{self, Comment},
    events::{Event, EventEnvelope},
};

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct AutoFetcher {
    #[serde(skip)]
    handle: Option<AbortHandle>,
}
impl AutoFetcher {
    pub fn enable(&mut self, url: String, tx: Sender<EventEnvelope>) {
        if self.handle.is_some() {
            return;
        }
        let join = tokio::task::spawn(async move {
            loop {
                let comments = comments::get_comments_from_url(&url, true)
                    .instrument(tracing::info_span!("comments_update_loop"))
                    .await;
                let _ = tx
                    .send(EventEnvelope {
                        event: Event::CommentsUpdate { comments },
                        span: tracing::info_span!("auto_fetch_comments"),
                    })
                    .await;
                let _ = tokio::time::sleep(Duration::from_secs(60)).await;
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

#[cfg(feature = "integration-tests")]
#[cfg(test)]
mod tests {

    #[tokio::test]
    async fn test_auto_fetcher_integration() {
        let url = std::env::var("TEST_HN_URL").expect("TEST_HN_URL must be set");
        assert!(!url.is_empty(), "TEST_HN_URL must not be empty");

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let mut fetcher = super::AutoFetcher::default();

        fetcher.enable(url, tx);

        if let Some(envelope) = rx.recv().await {
            if let crate::events::Event::CommentsUpdate { comments } = envelope.event {
                assert!(!comments.is_empty());
                assert!(comments.len() >= 1)
            }
        }

        fetcher.disable();
    }
}
