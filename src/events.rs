#![allow(unused)]
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, Sender};

use crate::{
    autofetcher::AutoFetcher,
    comments::Comment,
    common_gui::{Flags, ProcessingData},
    evaluation::{self, Evaluation, Usable},
    job_description::{self, JobDescriptions},
    notify::{self, NotifyData},
};

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct State {
    pub hydrated: bool,
    pub auto_fetcher: AutoFetcher,
    pub cache_key_error: Option<String>,
    pub evaluations: HashMap<u32, Evaluation>,
    pub comments: Vec<Comment>,
    pub job_descriptions: JobDescriptions,
    pub auto_fetch: bool,
    pub notify_data: NotifyData,
    pub eval_cache: Option<evaluation::EvaluationCache>,
    pub flags: HashMap<u32, Flags>,
    pub processing: ProcessingData,
    pub api_key: Arc<str>,
    pub barriers: HashMap<u32, Vec<Barrier>>,
}

#[derive(Eq, PartialEq, Hash, Serialize, Deserialize, Clone, Debug)]
pub enum BarrierData {
    JobDescription,
    Evaluation,
}
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Barrier {
    name: String,
    remaining: HashSet<BarrierData>,
    then: Vec<Event>,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum EvaluateEvent {
    Evaluate {
        comment: Comment,
        requirements: String,
        pdf_path: String,
    },
    EvaluateTry {
        comment: Comment,
        requirements: String,
        pdf_path: String,
        retry: usize,
    },
    EvaluationCacheFetch {
        requirements: String,
        pdf_path: String,
        then: Vec<Event>,
    },
    EvaluationEnrichWithJd {
        comment_id: u32,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum JobDescriptionEvent {
    FetchJobDescription {
        id: u32,
        model: String,
        input: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Event {
    EvaluateGroup(EvaluateEvent),
    JobDescriptionGroup(JobDescriptionEvent),
    Notify { id: u32 },
    Signal(u32, BarrierData),
}

pub struct EventHandler {
    pub state: Arc<RwLock<State>>,
    pub tx: Sender<Event>,
}
impl EventHandler {
    pub fn spawn(state: Arc<RwLock<State>>, api_key: String) -> Sender<Event> {
        let (tx, rx) = mpsc::channel(100);
        let event_handler = Self {
            state,
            tx: tx.clone(),
        };

        tokio::spawn(async move {
            event_handler.run(rx).await;
        });

        tx
    }
    async fn run(&self, mut rx: mpsc::Receiver<Event>) {
        let mut queue: VecDeque<Event> = VecDeque::new();

        loop {
            match rx.recv().await {
                Some(event) => queue.push_back(event),
                None => {
                    tracing::error!("event channel closed unexpectedly");
                    continue;
                }
            }

            while let Ok(event) = rx.try_recv() {
                queue.push_back(event);
            }

            while let Some(event) = queue.pop_front() {
                self.handle(event, &mut queue).await;
            }
        }
    }
    async fn handle(&self, event: Event, queue: &mut VecDeque<Event>) {
        use Event::*;
        match event {
            EvaluateGroup(e_event) => self.handle_evaluate(e_event, queue).await,
            JobDescriptionGroup(job_description_event) => todo!(),
            Signal(id, barrier_data) => {
                let mut state = self.state.write();
                let barriers = state.barriers.entry(id).or_default();
                barriers.retain_mut(|barrier| {
                    if barrier.remaining.remove(&barrier_data) {
                        if barrier.remaining.is_empty() {
                            barrier.then.iter().for_each(|e| {
                                queue.push_back(e.clone());
                            });
                            return false; // Remove
                        }
                    }
                    true // Keep
                });
            }
            Notify { id } => {
                let evaluation = self.state.read().evaluations.get(&id).cloned();
                let state = self.state.clone();
                tokio::spawn(async move {
                    if let Some(eval) = evaluation {
                        let mut state = state.write();
                        state.notify_data.notify_evaluation(id, &eval);
                    }
                });
            }
        }
    }
    async fn handle_evaluate(&self, event: EvaluateEvent, queue: &mut VecDeque<Event>) {
        use EvaluateEvent::*;
        match event {
            Evaluate {
                comment,
                requirements,
                pdf_path,
            } => {
                queue.push_front(Event::EvaluateGroup(EvaluateTry {
                    comment,
                    requirements,
                    pdf_path,
                    retry: 1,
                }));
            }
            EvaluateTry {
                comment,
                requirements,
                pdf_path,
                retry,
            } => {
                // Retry stop
                if retry > 3 {
                    return;
                }
                // If no EvalCache, try to get it and then retry event
                if !self.state.read().eval_cache.is_usable() {
                    let repeat_ev = {
                        let requirements = requirements.clone();
                        let pdf_path = pdf_path.clone();
                        Evaluate {
                            comment,
                            requirements,
                            pdf_path,
                        }
                    };
                    queue.push_back(Event::EvaluateGroup(EvaluationCacheFetch {
                        requirements,
                        pdf_path,
                        then: vec![Event::EvaluateGroup(repeat_ev)],
                    }))
                } else {
                    // EvalCache Available, processing with retry
                    let eval_cache = self.state.read().eval_cache.clone().unwrap();
                    let api_key = self.state.read().api_key.to_string();
                    let tx = self.tx.clone();
                    let state = self.state.clone();
                    // Process async
                    tokio::spawn(async move {
                        let result =
                            evaluation::evaluate_comment_cached(&comment, &eval_cache, &api_key)
                                .await;
                        match result {
                            Ok(ev) => {
                                let mut state = state.write();
                                state.evaluations.insert(comment.id, ev);
                                insert_jd_evaluation_barrier(&mut state, comment.id);
                                let _ =
                                    tx.send(Event::Signal(comment.id, BarrierData::JobDescription));
                            }
                            Err(err) => {
                                tracing::error!("Evaluation failed: {}", err);
                                let _ = tx
                                    .send(Event::EvaluateGroup(EvaluateTry {
                                        comment,
                                        requirements,
                                        pdf_path,
                                        retry: retry + 1,
                                    }))
                                    .await;
                            }
                        }
                    });
                }
            }
            EvaluationCacheFetch {
                requirements,
                pdf_path,
                then,
            } => {
                let api_key = self.state.read().api_key.clone();
                let tx = self.tx.clone();
                let state = self.state.clone();
                tokio::spawn(async move {
                    let path = PathBuf::from(pdf_path);
                    let ttl = Duration::from_hours(24);
                    let result =
                        evaluation::create_evaluation_cache(&api_key, &path, &requirements, ttl)
                            .await;
                    match result {
                        Ok(cache_key) => {
                            let cache = evaluation::EvaluationCache {
                                key: cache_key,
                                timestamp: chrono::Utc::now(),
                                ttl,
                            };

                            state.write().eval_cache = Some(cache);
                            for event in then {
                                let _ = tx.send(event).await;
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
            EvaluationEnrichWithJd { comment_id } => {
                let mut state = self.state.write();
                if let (Some(mut ev), Some(jd)) = (
                    state.evaluations.get_mut(&comment_id).cloned(),
                    state.job_descriptions.data.get(&comment_id).cloned(),
                ) {
                    ev.job_description = Some(jd);
                    // Remove the barrier as it is now satisfied
                    if let Some(barriers) = state.barriers.get_mut(&comment_id) {
                        barriers.retain(|b| b.name != "enrich_evaluation_with_jd");
                    }
                }
            }
        }
    }
    async fn handle_job_description(
        &self,
        event: JobDescriptionEvent,
        queue: &mut VecDeque<Event>,
    ) {
        match event {
            JobDescriptionEvent::FetchJobDescription { id, model, input } => {
                let llm_config = llmuxer::LlmConfig {
                    provider: llmuxer::Provider::Gemini,
                    api_key: self.state.read().api_key.to_string(),
                    base_url: None,
                    model: model,
                };
                let api_key = self.state.read().api_key.to_string().clone();
                let state = self.state.clone();
                let tx = self.tx.clone();

                tokio::spawn(async move {
                    for _ in 0..3 {
                        let job_result = state.write().job_descriptions.get(id, &input, &api_key);
                        let job_result =
                            job_description::parse_job_description(llm_config.clone(), &input);

                        match job_result {
                            Err(e) => tracing::error!("ParseJobDescription Error: {:?}", e),
                            Ok(jd) => {
                                let _ = tx.send(Event::Signal(id, BarrierData::JobDescription));
                                break;
                            }
                        }
                    }
                });
            }
        }
    }
}

pub fn insert_jd_evaluation_barrier(state: &mut State, id: u32) {
    let name = String::from("enrich_evaluation_with_jd");
    let barrier = {
        // Check if barrier already exist to not duplicate
        let barriers = state.barriers.get(&id).cloned().unwrap_or_default();
        if barriers.into_iter().any(|b| b.name == name) {
            return;
        }
        let jd = state.job_descriptions.data.get(&id);
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

        b.then = vec![
            EvaluateEvent::EvaluationEnrichWithJd { comment_id: id }.into_event(),
            Event::Notify { id: id },
        ];

        b
    };
    state.barriers.entry(id).or_default().push(barrier);
}

pub trait IntoEvent {
    fn into_event(self) -> Event;
}

impl IntoEvent for EvaluateEvent {
    fn into_event(self) -> Event {
        Event::EvaluateGroup(self)
    }
}

impl IntoEvent for JobDescriptionEvent {
    fn into_event(self) -> Event {
        Event::JobDescriptionGroup(self)
    }
}
