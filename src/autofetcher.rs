use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::{sync::mpsc::Sender, task::AbortHandle};

use crate::comments::{self, Comment};

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct AutoFetcher {
    #[serde(skip)]
    handle: Option<AbortHandle>,
}
impl AutoFetcher {
    pub fn enable(&mut self, url: String, tx: Sender<Vec<Comment>>) {
        if self.handle.is_some() {
            return;
        }
        let join = tokio::task::spawn(async move {
            loop {
                let comments = comments::get_comments_from_url(&url, true).await;
                let _ = tx.send(comments).await;
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

pub fn update_comments(
    comments_rx: &mut tokio::sync::mpsc::Receiver<Vec<Comment>>,
    comments: &mut Vec<Comment>,
) {
    if let Ok(new_comments) = comments_rx.try_recv() {
        *comments = new_comments;
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

        if let Some(comments) = rx.recv().await {
            assert!(!comments.is_empty());
            assert!(comments.len() >= 1)
        }

        fetcher.disable();
    }
}
