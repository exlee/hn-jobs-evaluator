#![allow(unused)]

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::{
    OwnedSemaphorePermit, Semaphore, SemaphorePermit,
    mpsc::{self, Sender},
};
use tracing::{Instrument as _, instrument};

use crate::{
    backend::{
        self,
        app_service::{self, AppService},
        autofetcher::AutoFetcher,
        batch_processor::BatchProcessor,
        comments::Comment,
        evaluation::{Evaluation, EvaluationCache},
        front_page::FrontPageProcessor,
        job_description::JobDescription,
        notify::{self, NotifyData},
    },
    common_gui::{Flags, ProcessingData},
    models::{AppServiceDefault, Story, Usable as _},
};

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct State {
    pub hydrated: bool,
    #[serde(skip)]
    pub auto_fetcher: AutoFetcher,
    pub batch_processor: BatchProcessor,
    #[serde(skip)]
    pub front_page_processor: FrontPageProcessor,
    pub cache_key_error: Option<String>,
    pub evaluations: HashMap<u32, Evaluation>,
    pub comments: Vec<Comment>,
    pub job_descriptions: HashMap<u32, JobDescription>,
    pub notify_data: NotifyData,
    pub eval_cache: Option<EvaluationCache>,
    pub flags: HashMap<u32, Flags>,
    pub processing: ProcessingData,
    pub api_key: Arc<str>,
    pub barriers: HashMap<u32, Vec<Barrier>>,
    pub notifications: bool,
}

#[derive(Eq, PartialEq, Hash, Serialize, Deserialize, Clone, Debug)]
pub enum BarrierData {
    JobDescription,
    Evaluation,
}
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Barrier {
    name: String,
    remaining: HashSet<BarrierData>,
    then: Vec<Event>,
}
pub struct EventEnvelope {
    pub event: Event,
    pub span: tracing::Span,
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Event<{}>", <&'static str>::from(self))
    }
}
pub struct EventHandler {
    pub state: Arc<RwLock<State>>,
    pub tx: Arc<RwLock<Sender<EventEnvelope>>>,
    pub api_semaphore: Arc<Semaphore>,
    pub app_service: Arc<dyn AppService>,
}
#[event_macros::event_processor]
impl EventHandler {
    pub fn sender(&self) -> mpsc::Sender<EventEnvelope> {
        self.tx.read().clone()
    }
    pub fn new(state: State, app_service: Arc<dyn AppService>) -> Arc<Self> {
        let arw_state = Arc::new(RwLock::new(state));
        tracing::debug!("spawning handler");
        let eha = Self::spawn(arw_state.clone(), app_service.clone());
        eha
    }
    pub async fn send(&self, env: EventEnvelope) {
        self.tx.read().send(env).await;
    }
    pub fn spawn(state: Arc<RwLock<State>>, app_service: Arc<dyn AppService>) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(100);
        let event_handler_str = Self {
            state,
            api_semaphore: new_api_semaphore(),
            tx: Arc::new(RwLock::new(tx.clone())),
            app_service: app_service,
        };
        let event_handler = Arc::new(event_handler_str);

        {
            let event_handler = event_handler.clone();
            tokio::spawn(async move {
                tracing::debug!("pre handler run");
                event_handler.run(rx).await;
                tracing::debug!("post handler run");
            });
        }

        event_handler
    }
    async fn run(&self, mut rx: mpsc::Receiver<EventEnvelope>) {
        let mut queue: VecDeque<EventEnvelope> = VecDeque::new();

        loop {
            match rx.recv().await {
                Some(envelope) => queue.push_back(envelope),
                None => {
                    tracing::error!("event channel closed unexpectedly");
                    break;
                }
            }

            while let Ok(envelope) = rx.try_recv() {
                queue.push_back(envelope);
            }

            while let Some(envelope) = queue.pop_front() {
                dbg!(&envelope.event);
                self.handle(envelope, &mut queue).await;
            }
        }
    }
    #[handler(SetNotifications)]
    fn process_set_notifications(&self, enabled: bool) {
        self.state.write().notifications = enabled;
    }
    #[handler(AutoFetchStart)]
    fn process_auto_fetch_start(&self, url: String) {
        self.state.write().auto_fetcher.enable(url, self.sender());
    }
    #[handler(AutoFetchStop)]
    fn process_auto_fetch_stop(&self) {
        self.state.write().auto_fetcher.disable();
    }
    #[handler(BatchProcessingStart)]
    fn process_batch_processing_start(&self, requirements: String, pdf_path: String) {
        self.state.write().batch_processor.enable(
            self.api_semaphore.clone(),
            self.state.clone(),
            self.sender(),
            requirements,
            pdf_path,
        );
    }
    #[handler(BatchProcessingStop)]
    fn process_batch_processing_stop(&self) {
        self.state.write().batch_processor.disable();
    }
    #[handler(CommentsProcess)]
    fn process_comments(&self, url: String) {
        let tx = self.sender();
        let span = env.span.clone();
        let app_service = self.app_service.clone();

        tokio::task::spawn(
            async move {
                let comments = app_service
                    .get_comments_from_url(url.clone(), true)
                    .instrument(tracing::info_span!(parent: &span, "comments_update_loop"))
                    .await;
                let _ = tx
                    .send(EventEnvelope {
                        event: Event::CommentsUpdate { comments },
                        span: tracing::info_span!("auto_fetch_comments"),
                    })
                    .await;
            }
            .instrument(env.span),
        );
    }
    #[handler(CommentsUpdate)]
    fn process_comments_update(&self, comments: Vec<Comment>) {
        let mut hm: HashMap<u32, Comment> = self
            .state
            .read()
            .comments
            .clone()
            .into_iter()
            .map(|c| (c.id, c))
            .collect();

        for c in comments {
            match hm.insert(c.id, c.clone()) {
                Some(old) => {
                    if old != c {
                        tracing::info!("Updated Comment: {:?}", c);
                    }
                }
                None => tracing::info!("New comment: {:?}", c),
            }
        }
        self.state.write().comments = hm.into_values().collect::<Vec<_>>();
    }
    #[handler(Signal)]
    fn process_signal(&self, id: u32, barrier_data: BarrierData, queue: &mut VecDeque<EventEnvelope>) {
        tracing::info!("Signal({}): {:?}", id, barrier_data);
        let mut state = self.state.write();
        let barriers = state.barriers.entry(id).or_default();
        barriers.retain_mut(|barrier| {
            if barrier.remaining.remove(&barrier_data) {
                tracing::debug!(
                    "Removed {:?} from {}. Remaining: {:?}",
                    barrier_data,
                    id,
                    barrier.remaining
                );
                if barrier.remaining.is_empty() {
                    tracing::debug!("Barrier emptied.");
                    queue.extend(barrier.then.drain(..).map(|event| EventEnvelope {
                        event,
                        span: tracing::info_span!("barrier_completion"),
                    }));
                    return false; // Remove
                }
            }
            true // Keep
        });
    }
    #[handler(FlagEventUpdate)]
    fn process_flag_update(&self, id: u32, flag: Flags) {
        self.state.write().flags.insert(id, flag);
    }
    #[handler(Notify)]
    fn process_notify(&self, id: u32, env: EventEnvelope) {
        if !self.state.read().notifications {
            self.state.write().notify_data.mark_notified(id);
            return;
        }
        let evaluation = self.state.read().evaluations.get(&id).cloned();
        let state = self.state.clone();
        let app_service = self.app_service.clone();
        tokio::spawn(async move {
            let _span = env.span.entered();
            let _guard = tracing::info_span!("notify evaluation");
            if let Some(eval) = evaluation {
                let mut state = state.write();
                app_service.notify_evaluation(id, state.notify_data.clone(), eval);
            }
        });
    }
    #[handler(Evaluate)]
    fn process_evaluate(
        &self,
        try_cache: bool,
        comment: Comment,
        requirements: String,
        pdf_path: String,
        permit: Option<Arc<OwnedSemaphorePermit>>,
    ) {
        {
            let _span = tracing::debug_span!("Create barrier").entered();
            let mut state = self.state.write();
            tracing::debug!("pre create");
            insert_jd_evaluation_barrier(&mut state, self.sender(), comment.id);
        }
        if try_cache && let Some(ev) = self.state.read().evaluations.get(&comment.id) {
            tracing::debug!("Found cached Evaluation: {}", comment.id);
            return;
        }
        let tx = self.sender();
        tokio::task::spawn(async move {
            tx.send(EventEnvelope {
                event: Event::EvaluateTry {
                    try_cache,
                    comment,
                    requirements,
                    pdf_path,
                    permit,
                    retry: 1,
                },
                span: tracing::info_span!("EvaluateTry"),
            })
            .await;
        });
    }
    #[handler(EvaluationCacheFetch)]
    fn process_evaluate_cache_fetch(&self, requirements: String, pdf_path: String, then: Vec<Event>) {
        let api_key = self.state.read().api_key.clone();
        let tx = self.sender();
        let state = self.state.clone();
        let app_service = self.app_service.clone();
        tokio::spawn(async move {
            let path = PathBuf::from(pdf_path);
            let ttl = Duration::from_hours(24);
            let api_key = api_key.clone().to_string();
            let result = app_service
                .create_evaluation_cache(api_key, path, requirements, ttl)
                .await;
            match result {
                Ok(cache_key) => {
                    let cache = EvaluationCache {
                        key: cache_key,
                        timestamp: chrono::Utc::now(),
                        ttl,
                    };

                    state.write().eval_cache = Some(cache);
                    for event in then {
                        let _ = tx
                            .send(EventEnvelope {
                                event,
                                span: tracing::info_span!("queued_event"),
                            })
                            .await;
                    }
                }
                Err(err) => {
                    tracing::error!("FetchEvalCache error: {}", err);
                    state.write().eval_cache = None;
                    for event in then {
                        tracing::error!("Dropping Event: {:?}", event);
                    }
                }
            };
        });
    }
    #[handler(EvaluationEnrichWithJd)]
    fn process_evaluate_enrich_with_jd(&self, comment_id: u32) {
        let mut state = self.state.write();
        if let (Some(mut ev), Some(jd)) = (
            state.evaluations.get_mut(&comment_id).cloned(),
            state.job_descriptions.get(&comment_id).cloned(),
        ) {
            ev.job_description = Some(jd);
            state.evaluations.insert(comment_id, ev);
            // Remove the barrier as it is now satisfied
            if let Some(barriers) = state.barriers.get_mut(&comment_id) {
                barriers.retain(|b| b.name != "enrich_evaluation_with_jd");
            }
        }
    }
    #[handler(FetchJobDescription)]
    fn process_job_description_fetch(
        &self,
        id: u32,
        model: String,
        input: String,
        try_cache: bool,
        permit: Option<Arc<OwnedSemaphorePermit>>,
    ) {
        {
            let mut state = self.state.write();
            insert_jd_evaluation_barrier(&mut state, self.sender(), id);
        }
        let llm_config = llmuxer::LlmConfig {
            provider: llmuxer::Provider::Gemini,
            api_key: self.state.read().api_key.to_string(),
            base_url: None,
            model: model,
        };
        if try_cache && let Some(jd) = self.state.read().job_descriptions.get(&id) {
            tracing::debug!("Found cached JobDescription: {}", id);
            return;
        }
        let api_key = self.state.read().api_key.to_string().clone();
        let state = self.state.clone();
        let tx = self.sender();
        let app_service = self.app_service.clone();

        tokio::spawn(async move {
            let permit = permit;
            for _ in 0..3 {
                let app_service = app_service.clone();
                //let job_result = state.write().job_descriptions.get(id, &input, &api_key);

                let input = input.clone();
                let llm_config = llm_config.clone();
                let job_result =
                    tokio::task::spawn_blocking(move || app_service.parse_job_description(llm_config, input)).await;

                match job_result {
                    Err(e) => tracing::error!("Join Error: {:?}", e),
                    Ok(Err(e)) => tracing::error!("ParseJobDescription Error: {:?}", e),
                    Ok(Ok(jd)) => {
                        {
                            let mut state = state.write();
                            state.job_descriptions.insert(id, jd);
                        }
                        let _ = tx
                            .send(EventEnvelope {
                                event: Event::Signal {
                                    id,
                                    barrier_data: BarrierData::JobDescription,
                                },
                                span: tracing::info_span!("signal_job_description"),
                            })
                            .await;
                        break;
                    }
                }
            }
        });
    }

    #[handler(RemoveNotify)]
    fn process_remove_notify(&self, id: u32) {
        tracing::debug!("Remove ID from Notify");
        self.state.write().notify_data.notified_ids.remove(&id);
    }
    #[handler(RemoveEvaluationAll)]
    fn process_remove_evaluation_all(&self) {
        tracing::info!("Remove all evaluation disabled");
        // self.state.write().evaluations.clear();
    }
    #[handler(SyncApiKey)]
    fn process_sync_apikey(&self, key: String) {
        tracing::debug!("Sync ApiKey");
        self.state.write().api_key = Arc::from(key);
    }
    #[handler(EvaluateTry)]
    fn process_evaluate_try(
        &self,
        try_cache: bool,
        comment: Comment,
        requirements: String,
        pdf_path: String,
        retry: usize,
        permit: Option<Arc<OwnedSemaphorePermit>>,
    ) {
        tracing::debug!("EvaluateTry(...)");
        // Retry stop
        if retry > 3 {
            tracing::debug!("Max tries reached.");
            return;
        }
        // If no EvalCache, try to get it and then retry event
        if !self.state.read().eval_cache.is_usable() {
            let repeat_ev = {
                let requirements = requirements.clone();
                let pdf_path = pdf_path.clone();
                Event::Evaluate {
                    try_cache,
                    comment,
                    requirements,
                    pdf_path,
                    permit,
                }
            };
            queue.push_back(EventEnvelope {
                event: Event::EvaluationCacheFetch {
                    requirements,
                    pdf_path,
                    then: vec![repeat_ev],
                },
                span: tracing::info_span!("EvaluationCacheFetch"),
            });
        } else {
            // EvalCache Available, processing with retry
            let eval_cache = self.state.read().eval_cache.clone().unwrap();
            let api_key = self.state.read().api_key.to_string();
            let tx = self.sender();
            let state = self.state.clone();
            let app_service = Arc::clone(&self.app_service);
            // Process async
            tokio::spawn(
                async move {
                    let result = app_service
                        .evaluate_comment_cached(comment.clone(), eval_cache, api_key)
                        .await;
                    match result {
                        Ok(ev) => {
                            {
                                let mut state = state.write();
                                state.evaluations.insert(comment.id, ev);
                            }
                            let _ = tx
                                .send(EventEnvelope {
                                    event: Event::Signal {
                                        id: comment.id,
                                        barrier_data: BarrierData::Evaluation,
                                    },
                                    span: tracing::info_span!("signal_evaluation"),
                                })
                                .await;
                        }
                        Err(err) => {
                            tracing::error!("Evaluation failed: {}", err);
                            let _ = tx
                                .send(EventEnvelope {
                                    event: Event::EvaluateTry {
                                        try_cache,
                                        comment,
                                        requirements,
                                        pdf_path,
                                        retry: retry + 1,
                                        permit,
                                    },
                                    span: tracing::info_span!("retry_evaluate"),
                                })
                                .await;
                        }
                    };
                }
                .instrument(tracing::info_span!(parent: &env.span, "request")),
            );
        }
    }
    #[handler(FrontPageProcessingStart)]
    fn process_front_page_processing_start(&self) {
        self.state.write().front_page_processor.enable(self.sender());
    }

    #[handler(FrontPageProcessingEnd)]
    fn process_front_page_processing_end(&self) {
        self.state.write().front_page_processor.disable();
    }

    #[handler(FrontPageUpdate)]
    fn process_front_page_update(&self, stories: Vec<Story>) {
        let topic = self.state.read().notify_data.topic.clone();
        let story_check: fn(String) -> bool = |text: String| {
            let checks = vec!["who is hiring", "who's hiring"];
            for check in checks {
                if text.to_lowercase().contains(check) {
                    return true;
                }
            }
            return false;
        };
        if stories.iter().any(|s| story_check(s.title.clone().unwrap_or_default())) {
            let title = "\"HN: Who is hiring\" topic found";
            tokio::spawn(notify::ntfy_notify(topic, title.into(), "".into()));
            self.send(EventEnvelope {
                event: Event::FrontPageProcessingEnd,
                span: env.span,
            });
        }
    }
}

impl Serialize for EventHandler {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.state.read().serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for EventHandler {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let state = State::deserialize(deserializer)?;

        let arc_eh = EventHandler::new(state, Arc::new(AppServiceDefault {}));
        if let Ok(eh) = Arc::try_unwrap(arc_eh) {
            Ok(eh)
        } else {
            Err(serde::de::Error::custom("can't unwrap arc"))
        }
    }
}
impl Default for EventHandler {
    fn default() -> Self {
        let (tx, _) = mpsc::channel(100);
        Self {
            state: Arc::new(RwLock::new(State::default())),
            api_semaphore: new_api_semaphore(),
            app_service: Arc::new(app_service::AppServiceDefault {}),
            tx: Arc::new(RwLock::new(tx)),
        }
    }
}
pub fn new_api_semaphore() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(5))
}

#[tracing::instrument(skip(state, tx))]
pub fn insert_jd_evaluation_barrier(state: &mut State, tx: Sender<EventEnvelope>, id: u32) {
    tracing::debug!("Creating enrich_evaluation_with_jd barrier");
    let name = String::from("enrich_evaluation_with_jd");
    let barrier = {
        // Check if barrier already exist to not duplicate
        let barriers = state.barriers.get(&id).cloned().unwrap_or_default();
        if barriers.into_iter().any(|b| b.name == name) {
            return;
        }
        let jd = state.job_descriptions.get(&id);
        let ev = state.evaluations.get(&id);

        // Check what we still need
        let mut b = Barrier::default();
        b.name = name;
        if jd.is_none() {
            b.remaining.insert(BarrierData::JobDescription);
        }
        if ev.is_none() {
            b.remaining.insert(BarrierData::Evaluation);
        }

        let events = vec![
            Event::EvaluationEnrichWithJd { comment_id: id },
            Event::Notify { id: id },
        ];
        if b.remaining.is_empty() {
            for e in events {
                let _ = tx.try_send(EventEnvelope {
                    event: e.clone(),
                    span: tracing::info_span!("event", event = ?e),
                });
            }
            return;
        }

        b.then = events;

        b
    };
    state.barriers.entry(id).or_default().push(barrier);
}
