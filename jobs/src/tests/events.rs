use crate::{backend::app_service::Blank as _, events::*, models::Comment, tests::app_service::AppServiceClosures};

use parking_lot::RwLock;
use std::{marker::PhantomData, sync::Arc, time::Duration};
use tokio::sync::mpsc;

pub struct ChannelLogger<T, R> {
    f1: PhantomData<T>,
    f2: PhantomData<R>,
}
impl<T: 'static + Sync + Send, R: 'static + Clone + Sync + Send + std::fmt::Debug> ChannelLogger<T, R> {
    async fn forward_msg(
        container: Arc<RwLock<Vec<R>>>,
        mut inner_rx: tokio::sync::mpsc::Receiver<T>,
        inner_tx: tokio::sync::mpsc::Sender<T>,
        transformer: Box<dyn Fn(&T) -> R + Send>,
    ) {
        while let Some(message_raw) = inner_rx.recv().await {
            let message = transformer(&message_raw);
            dbg!(&message);
            container.write().push(message); // Clone to store and send
            if inner_tx.send(message_raw).await.is_err() {
                // The receiving end has been dropped, so we can stop forwarding.
                break;
            }
        }
    }
    fn make(
        inner_tx: tokio::sync::mpsc::Sender<T>,
        transformer: Box<dyn Fn(&T) -> R + Send>,
    ) -> (mpsc::Sender<T>, Arc<RwLock<Vec<R>>>) {
        let (tx, rx) = mpsc::channel(100); // Using mpsc::channel for internal communication
        let container = Arc::new(RwLock::new(Vec::new()));
        let container_clone = Arc::clone(&container);

        tokio::spawn(async move {
            Self::forward_msg(container_clone, rx, inner_tx, transformer).await;
        });

        (tx, container)
    }
}
struct TestHarness {
    handler: Arc<EventHandler>,
    event_tx: mpsc::Sender<EventEnvelope>,
    container: Arc<RwLock<Vec<Event>>>,
}

impl TestHarness {
    fn new() -> Self {
        let _ = tracing_subscriber::fmt()
            .with_test_writer() // captures per-test instead of global stderr
            .with_env_filter("debug")
            .try_init();
        let app_service = AppServiceClosures::default();
        let state = State::default();
        let handler = EventHandler::new(state, Arc::new(app_service));
        let tx = handler.tx.read().clone();
        let (log_tx, container) = ChannelLogger::make(tx, Box::new(|e: &EventEnvelope| e.event.clone()));
        {
            let handler = handler.clone();
            let mut write_tx = handler.tx.write();
            *write_tx = log_tx.clone();
        }

        // Assuming EventHandler::new() initializes default state
        Self {
            handler,
            event_tx: log_tx,
            container: container.clone(),
        }
    }
    async fn send(&self, event: Event) {
        let _ = self.event_tx.send(EventEnvelope {
            event,
            span: tracing::Span::none(),
        });
    }
}

#[tokio::test]
async fn test_event_handler_initialization() {
    let harness = TestHarness::new();
    assert!(harness.handler.state.read().hydrated == false);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_evaluation_event_triggers_try() {
    let harness = TestHarness::new();

    let event = Event::Evaluate {
        try_cache: true,
        comment: Comment::blank(),
        requirements: "reqs".to_string(),
        pdf_path: "path".to_string(),
        permit: None,
    };
    let _ = harness
        .event_tx
        .send(EventEnvelope {
            event,
            span: tracing::debug_span!("test_evaluation_event_triggers_try"),
        })
        .await
        .expect("EventEnvelope can't be sent");

    assert!(harness.handler.clone().state.read().evaluations.len() == 0);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(harness.handler.clone().state.read().evaluations.len() == 1);

    assert!(
        harness
            .container
            .read()
            .iter()
            .any(|e| matches!(e, Event::EvaluateTry { .. })),
        "expected EvaluateTry, got: {:?}",
        harness.container.read().clone()
    );
}
