use std::{sync::Arc, time::Duration};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{Semaphore, mpsc::Sender},
    task::AbortHandle,
};
use tracing::Instrument as _;

use crate::{
    backend::evaluation::MODEL,
    events::{self, Event, EventEnvelope},
};

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct BatchProcessor {
    #[serde(skip)]
    handle: Option<AbortHandle>,
}
impl BatchProcessor {
    pub fn enable(
        &mut self,
        semaphore: Arc<Semaphore>,
        state: Arc<RwLock<events::State>>,
        tx: Sender<EventEnvelope>,
        requirements: String,
        pdf_path: String,
    ) {
        if self.handle.is_some() {
            return;
        }
        let join = tokio::task::spawn(async move {
            loop {
                let (comments, evaluations) = {
                    let state = state.read();
                    (state.comments.clone(), state.evaluations.clone())
                };

                async {
                    for comment in comments {
                        if let Ok(permit) = semaphore.clone().acquire_owned().await {
                            let permit = Arc::new(permit);
                            if let Some(ev) = evaluations.get(&comment.id) {
                                if ev.job_description.is_none() && comment.text.is_some() {
                                    let _ = tx
                                        .send(EventEnvelope {
                                            event: Event::FetchJobDescription {
                                                try_cache: true,
                                                id: comment.id,
                                                model: String::from(MODEL),
                                                input: comment.clone().text.unwrap(),
                                                permit: Some(permit.clone()),
                                            },
                                            span: tracing::info_span!("job_description"),
                                        })
                                        .await;
                                } else {
                                    continue;
                                }
                            }
                            let _ = tx
                                .send(EventEnvelope {
                                    event: Event::Evaluate {
                                        try_cache: false,
                                        comment: comment,
                                        permit: Some(permit.clone()),
                                        requirements: requirements.clone(),
                                        pdf_path: pdf_path.clone(),
                                    },
                                    span: tracing::info_span!("evaluate_comment"),
                                })
                                .await;
                        } else {
                            // Semaphore closed, stop processing
                            return;
                        }
                    }
                    let _ = tokio::time::sleep(Duration::from_secs(5)).await;
                }
                .instrument(tracing::info_span!("comments processing loop"))
                .await
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
