use std::path::PathBuf;
use std::time::Duration;

use crate::backend::comments::Comment;
use crate::backend::evaluation::{Evaluation, EvaluationCache};
use crate::backend::front_page::Story;
use crate::backend::job_description::JobDescription;
use crate::backend::notify::NotifyData;
use chrono::Utc;

#[macro_export]
macro_rules! async_res {
    ($lt:lifetime, $out:ty) => {
        std::pin::Pin<Box<dyn std::future::Future<Output = $out> + Send + $lt>>
    };
    ($out:ty) => {
        std::pin::Pin<Box<dyn std::future::Future<Output = $out> + Send + '_>>
    };
}
pub use async_res;

#[allow(unused)]
pub trait Blank {
    fn blank() -> Self;
}

impl Blank for JobDescription {
    fn blank() -> Self {
        JobDescription::default()
    }
}

impl Blank for Comment {
    fn blank() -> Self {
        Comment {
            created_at: Utc::now(),
            id: 0,
            author: String::new(),
            text: None,
            parent: 0,
            children: Vec::new(),
        }
    }
}

#[event_macros::service_handler]
pub trait AppService: Send + Sync {
    #[function(crate::backend::comments::get_comments_from_url)]
    #[blank(Vec::new())]
    async fn get_comments_from_url(&self, url: String, force: bool) -> Vec<Comment>;

    #[function(crate::backend::evaluation::evaluate_comment_cached)]
    #[blank(Ok(Evaluation::default()))]
    async fn evaluate_comment_cached(
        &self,
        comment: Comment,
        ev_cache: EvaluationCache,
        api_key: String,
    ) -> anyhow::Result<Evaluation>;

    #[function(crate::backend::evaluation::create_evaluation_cache)]
    #[blank(Ok(String::new()))]
    async fn create_evaluation_cache(
        &self,
        api_key: String,
        pdf_path: PathBuf,
        requirements: String,
        ttl: Duration,
    ) -> Result<String, String>;

    #[function(crate::backend::job_description::parse_job_description)]
    #[blank(Ok(JobDescription::default()))]
    fn parse_job_description(&self, llm_config: llmuxer::LlmConfig, input: String) -> Result<JobDescription, String>;

    #[function(notify_evaluation)]
    #[blank(Ok(()))]
    fn notify_evaluation(&self, id: u32, notify_data: NotifyData, evaluation: Evaluation) -> anyhow::Result<()>;

    #[function(crate::backend::front_page::get_front_page_stories)]
    #[blank(Ok(Vec::new()))]
    async fn get_front_page_stories(&self) -> anyhow::Result<Vec<Story>>;
}

impl std::fmt::Debug for dyn AppService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppService")
    }
}

fn notify_evaluation(id: u32, mut notify_data: NotifyData, evaluation: Evaluation) -> anyhow::Result<()> {
    notify_data.notify_evaluation(id, &evaluation)
}
