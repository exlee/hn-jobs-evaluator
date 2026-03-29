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
    autofetcher::AutoFetcher,
    batch_processor::BatchProcessor,
    comments::{self, Comment},
    common_gui::{Flags, ProcessingData},
    evaluation::{self, Evaluation, Usable},
    job_description::{self, JobDescriptions},
    notify::{self, NotifyData},
};

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct State {
    pub hydrated: bool,
    pub auto_fetcher: AutoFetcher,
    pub batch_processor: BatchProcessor,
    pub cache_key_error: Option<String>,
    pub evaluations: HashMap<u32, Evaluation>,
    pub comments: Vec<Comment>,
    pub job_descriptions: HashMap<u32, job_description::JobDescription>,
    pub notify_data: NotifyData,
    pub eval_cache: Option<evaluation::EvaluationCache>,
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
// Delete Evaluations

#[derive(Serialize, Deserialize, Clone )]
#[rustfmt::skip]
#[derive(strum::IntoStaticStr)]
pub enum Event {
    AutoFetchStart(String),
    AutoFetchStop,
    BatchProcessingStart(String, String),
    BatchProcessingStop,
    CommentsUpdate{ comments: Vec<Comment> },
    Evaluate { try_cache: bool, comment: Comment, requirements: String, pdf_path: String, #[serde(skip)] permit: Option<Arc<OwnedSemaphorePermit>> },
    EvaluateTry { try_cache: bool, comment: Comment, requirements: String, pdf_path: String, retry: usize, #[serde(skip)] permit: Option<Arc<OwnedSemaphorePermit>> },
    EvaluationCacheFetch { requirements: String, pdf_path: String, then: Vec<Event> },
    EvaluationEnrichWithJd { comment_id: u32 },
    FetchJobDescription { try_cache: bool, id: u32, model: String, input: String, #[serde(skip)] permit: Option<Arc<OwnedSemaphorePermit>> },
    FlagEventUpdate { id: u32, flag: Flags },
    Notify { id: u32 },
    CommentsProcess { url: String },
    RemoveEvaluationAll,
    RemoveNotify(u32),
    Signal(u32, BarrierData),
    SetNotifications{enabled: bool},
    SyncApiKey(String),
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
macro_rules! event_fn {
    ($name:ident(&$self:ident, $queue:ident) @ $pat:pat => $body:block ) => {
        paste::paste! {
            fn [<process_ $name>](&$self, e: Event, $queue: &mut VecDeque<Event>) {
                let $pat = e else { return };
                $body
            }
        }
    };
}
macro_rules! ev_guard {
    ($env:ident => $pat:pat) => {
        use Event::*;
        let $pat = $env.event else { return };
    };
}
pub struct EventHandler {
    pub state: Arc<RwLock<State>>,
    pub tx: Sender<EventEnvelope>,
    pub api_semaphore: Arc<Semaphore>,
}
impl EventHandler {
    pub fn new(state: State) -> Self {
        let arw_state = Arc::new(RwLock::new(state));
        tracing::debug!("spawning handler");
        let tx = Self::spawn(arw_state.clone());
        Self {
            state: arw_state,
            api_semaphore: new_api_semaphore(),
            tx,
        }
    }
    pub fn spawn(state: Arc<RwLock<State>>) -> Sender<EventEnvelope> {
        let (tx, rx) = mpsc::channel(100);
        let event_handler = Self {
            state,
            api_semaphore: new_api_semaphore(),
            tx: tx.clone(),
        };

        tokio::spawn(async move {
            tracing::debug!("pre handler run");
            event_handler.run(rx).await;
            tracing::debug!("post handler run");
        });

        tx
    }
    async fn run(&self, mut rx: mpsc::Receiver<EventEnvelope>) {
        let mut queue: VecDeque<EventEnvelope> = VecDeque::new();

        loop {
            match rx.recv().await {
                Some(envelope) => queue.push_back(envelope),
                None => {
                    tracing::error!("event channel closed unexpectedly");
                    continue;
                }
            }

            while let Ok(envelope) = rx.try_recv() {
                queue.push_back(envelope);
            }

            while let Some(envelope) = queue.pop_front() {
                self.handle(envelope, &mut queue).await;
            }
        }
    }
    #[rustfmt::skip]
    #[tracing::instrument(skip_all, parent=&env.span)]
    async fn handle(&self, env: EventEnvelope, queue: &mut VecDeque<EventEnvelope>) {
        use Event::*;
        tracing::debug!("handle: {:?}", env.event);
        match env.event {
            AutoFetchStart(_)=>self.process_auto_fetch_start(env,queue),
            AutoFetchStop=>self.process_auto_fetch_stop(env,queue),
            BatchProcessingStart(..)=>self.process_batch_processing_start(env,queue),
            BatchProcessingStop=>self.process_batch_processing_stop(env,queue),
            CommentsProcess{..} => self.process_comments(env, queue),
            CommentsUpdate{..}=>self.process_comments_update(env,queue),
            EvaluateTry{..}=>self.process_evaluate_try(env,queue),
            Evaluate{..}=>self.process_evaluate(env,queue),
            EvaluationCacheFetch{..}=>self.process_evaluate_cache_fetch(env,queue),
            EvaluationEnrichWithJd{..}=>self.process_evaluate_enrich_with_jd(env,queue),
            FetchJobDescription{..}=>self.process_job_description_fetch(env,queue),
            FlagEventUpdate{..}=>self.process_flag_update(env,queue),
            Notify{..}=>self.process_notify(env,queue),
            RemoveEvaluationAll=>self.process_remove_evaluation_all(env,queue),
            RemoveNotify(_)=>self.process_remove_notify(env,queue),
            SetNotifications { enabled } => self.state.write().notifications = enabled,
            Signal(..)=>self.process_signal(env,queue),
            SyncApiKey(_)=>self.process_sync_apikey(env,queue),
        }
    }
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_auto_fetch_start(&self, env: EventEnvelope, _queue: &mut VecDeque<EventEnvelope>) {
        ev_guard!(env => AutoFetchStart(url));
        self.state.write().auto_fetcher.enable(url, self.tx.clone());
    }
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_auto_fetch_stop(&self, env: EventEnvelope, _queue: &mut VecDeque<EventEnvelope>) {
        ev_guard!(env => AutoFetchStop);
        self.state.write().auto_fetcher.disable();
    }
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_batch_processing_start(
        &self,
        env: EventEnvelope,
        queue: &mut VecDeque<EventEnvelope>,
    ) {
        ev_guard!(env => BatchProcessingStart(requirements, pdf_path));

        self.state.write().batch_processor.enable(
            self.api_semaphore.clone(),
            self.state.clone(),
            self.tx.clone(),
            requirements,
            pdf_path,
        );
    }
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_batch_processing_stop(
        &self,
        env: EventEnvelope,
        queue: &mut VecDeque<EventEnvelope>,
    ) {
        ev_guard!(env => BatchProcessingStop);

        self.state.write().batch_processor.disable();
    }
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_comments(&self, env: EventEnvelope, queue: &mut VecDeque<EventEnvelope>) {
        ev_guard!(env => CommentsProcess { url } );
        let tx = self.tx.clone();
        tokio::task::spawn(
            async move {
                let comments = comments::get_comments_from_url(&url, true)
                    .instrument(tracing::info_span!("comments_update_loop"))
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
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_comments_update(&self, env: EventEnvelope, queue: &mut VecDeque<EventEnvelope>) {
        ev_guard!(env => CommentsUpdate { comments });

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
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_signal(&self, env: EventEnvelope, queue: &mut VecDeque<EventEnvelope>) {
        ev_guard!(env => Signal(id,barrier_data));
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
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_flag_update(&self, env: EventEnvelope, queue: &mut VecDeque<EventEnvelope>) {
        ev_guard!(env => FlagEventUpdate { id, flag });
        self.state.write().flags.insert(id, flag);
    }
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_notify(&self, env: EventEnvelope, queue: &mut VecDeque<EventEnvelope>) {
        ev_guard!(env => Notify { id });
        if !self.state.read().notifications {
            self.state.write().notify_data.mark_notified(id);
        }
        let evaluation = self.state.read().evaluations.get(&id).cloned();
        let state = self.state.clone();
        tokio::spawn(async move {
            let _span = env.span.entered();
            let _guard = tracing::info_span!("notify evaluation");
            if let Some(eval) = evaluation {
                let mut state = state.write();
                state.notify_data.notify_evaluation(id, &eval);
            }
        });
    }
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_evaluate(&self, env: EventEnvelope, queue: &mut VecDeque<EventEnvelope>) {
        ev_guard!(env => Evaluate { try_cache, comment, requirements, pdf_path, permit });

        {
            let _span = tracing::debug_span!("Create barrier").entered();
            let mut state = self.state.write();
            tracing::debug!("pre create");
            insert_jd_evaluation_barrier(&mut state, self.tx.clone(), comment.id);
        }
        if try_cache && let Some(ev) = self.state.read().evaluations.get(&comment.id) {
            tracing::debug!("Found cached Evaluation: {}", comment.id);
            return;
        }
        queue.push_front(EventEnvelope {
            event: Event::EvaluateTry {
                try_cache,
                comment,
                requirements,
                pdf_path,
                permit,
                retry: 1,
            },
            span: tracing::info_span!("EvaluateTry"),
        });
    }
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_evaluate_cache_fetch(
        &self,
        env: EventEnvelope,
        queue: &mut VecDeque<EventEnvelope>,
    ) {
        ev_guard!(env => EvaluationCacheFetch { requirements, pdf_path, then });

        let api_key = self.state.read().api_key.clone();
        let tx = self.tx.clone();
        let state = self.state.clone();
        tokio::spawn(async move {
            let path = PathBuf::from(pdf_path);
            let ttl = Duration::from_hours(24);
            let result =
                evaluation::create_evaluation_cache(&api_key, &path, &requirements, ttl).await;
            match result {
                Ok(cache_key) => {
                    let cache = evaluation::EvaluationCache {
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
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_evaluate_enrich_with_jd(
        &self,
        env: EventEnvelope,
        queue: &mut VecDeque<EventEnvelope>,
    ) {
        ev_guard!(env => EvaluationEnrichWithJd { comment_id });
        let mut state = self.state.write();
        if let (Some(mut ev), Some(jd)) = (
            state.evaluations.get_mut(&comment_id).cloned(),
            state.job_descriptions.get(&comment_id).cloned(),
        ) {
            ev.job_description = Some(jd);
            // Remove the barrier as it is now satisfied
            if let Some(barriers) = state.barriers.get_mut(&comment_id) {
                barriers.retain(|b| b.name != "enrich_evaluation_with_jd");
            }
        }
    }
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_job_description_fetch(
        &self,
        env: EventEnvelope,
        queue: &mut VecDeque<EventEnvelope>,
    ) {
        ev_guard!(env => FetchJobDescription { id, model, input, try_cache, permit});
        {
            let mut state = self.state.write();
            insert_jd_evaluation_barrier(&mut state, self.tx.clone(), id);
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
        let tx = self.tx.clone();

        tokio::spawn(async move {
            let permit = permit;
            for _ in 0..3 {
                //let job_result = state.write().job_descriptions.get(id, &input, &api_key);

                let input = input.clone();
                let llm_config = llm_config.clone();
                let job_result = tokio::task::spawn_blocking(move || {
                    job_description::parse_job_description(llm_config, &input)
                })
                .await;

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
                                event: Event::Signal(id, BarrierData::JobDescription),
                                span: tracing::info_span!("signal_job_description"),
                            })
                            .await;
                        break;
                    }
                }
            }
        });
    }

    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_remove_notify(&self, env: EventEnvelope, queue: &mut VecDeque<EventEnvelope>) {
        ev_guard!(env => RemoveNotify(id));
        tracing::debug!("Remove ID from Notify");
        self.state.write().notify_data.notified_ids.remove(&id);
    }
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_remove_evaluation_all(
        &self,
        env: EventEnvelope,
        queue: &mut VecDeque<EventEnvelope>,
    ) {
        ev_guard!(env => RemoveEvaluationAll);
        tracing::info!("Remove all evaluation disabled");
        // self.state.write().evaluations.clear();
    }
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_sync_apikey(&self, env: EventEnvelope, queue: &mut VecDeque<EventEnvelope>) {
        ev_guard!(env => SyncApiKey(key));
        tracing::debug!("Sync ApiKey");
        self.state.write().api_key = Arc::from(key);
    }
    #[tracing::instrument(skip_all, parent=&env.span)]
    fn process_evaluate_try(&self, env: EventEnvelope, queue: &mut VecDeque<EventEnvelope>) {
        ev_guard!(
            env => EvaluateTry {
                try_cache,
                comment,
                requirements,
                pdf_path,
                retry,
                permit,
            }
        );

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
            let tx = self.tx.clone();
            let state = self.state.clone();
            // Process async
            tokio::spawn(async move {
                let result =
                    evaluation::evaluate_comment_cached(&comment, &eval_cache, &api_key).await;
                match result {
                    Ok(ev) => {
                        {
                            let mut state = state.write();
                            state.evaluations.insert(comment.id, ev);
                        }
                        let _ = tx
                            .send(EventEnvelope {
                                event: Event::Signal(comment.id, BarrierData::Evaluation),
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
        Ok(EventHandler::new(state))
    }
}
impl Default for EventHandler {
    fn default() -> Self {
        let (tx, _) = mpsc::channel(100);
        Self {
            state: Arc::new(RwLock::new(State::default())),
            api_semaphore: new_api_semaphore(),
            tx,
        }
    }
}
pub fn new_api_semaphore() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(1))
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
