use std::{sync::Arc, time::Duration};

use tokio::{sync::mpsc::Sender, task::AbortHandle};
use tracing::Instrument as _;

const REFETCH_WAIT_SECONDS: u64 = 60;

use crate::{
    backend::app_service::{AppService, AppServiceDefault},
    demo::AppServiceDemo,
    events::{Event, EventEnvelope},
};

#[derive(Debug, Clone)]
pub struct AutoFetcher {
    pub app_service: Arc<dyn AppService>,
    pub handle: Option<AbortHandle>,
}

impl Default for AutoFetcher {
    fn default() -> Self {
        let app_service: Arc<dyn AppService> = if crate::demo::is_demo() {
            Arc::new(AppServiceDemo {})
        } else {
            Arc::new(AppServiceDefault {})
        };
        Self {
            app_service,
            handle: None,
        }
    }
}
impl AutoFetcher {
    pub fn enable(&mut self, url: String, tx: Sender<EventEnvelope>) {
        if self.handle.is_some() {
            return;
        }
        let app_service = Arc::clone(&self.app_service);
        let join = tokio::task::spawn(async move {
            loop {
                comment_fetch_iteration(&url, &tx, &app_service).await
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

#[tracing::instrument(skip_all, fields(url = %url))]
async fn comment_fetch_iteration(url: &String, tx: &Sender<EventEnvelope>, app_service: &Arc<dyn AppService>) {
    let comments = app_service
        .get_comments_from_url(url.clone(), true)
        .instrument(tracing::info_span!("comments_update_loop"))
        .await;
    let _ = tx
        .send(EventEnvelope {
            event: Event::CommentsUpdate { comments },
            span: tracing::info_span!("auto_fetch_comments"),
        })
        .await;
    let wait_millis = (REFETCH_WAIT_SECONDS * 1000) + (rand::random::<u64>() % (REFETCH_WAIT_SECONDS * 200))
        - (REFETCH_WAIT_SECONDS * 100);
    tracing::info!("waiting {:.2}s", wait_millis as f64 / 1000.0);
    let sleep_duration = Duration::from_millis(wait_millis);
    let _ = tokio::time::sleep(sleep_duration)
        .instrument(tracing::info_span!("sleep"))
        .await;
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
