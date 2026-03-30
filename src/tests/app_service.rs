use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::backend::app_service::Blank;
use crate::backend::comments::Comment;
use crate::backend::evaluation::{Evaluation, EvaluationCache};
use crate::backend::job_description::JobDescription;
use crate::backend::notify::NotifyData;
use crate::models::AppService;
use crate::models::async_res;
pub struct AppServiceClosures {
    pub get_comments_from_url: Arc<dyn Fn(&str, bool) -> Vec<Comment> + Send + Sync>,
    pub evaluate_comment_cached:
        Arc<dyn for<'a> Fn(&'a Comment, &'a EvaluationCache, &'a str) -> anyhow::Result<Evaluation> + Send + Sync>,
    pub create_evaluation_cache:
        Arc<dyn for<'a> Fn(&'a str, &'a Path, &'a str, Duration) -> Result<String, String> + Send + Sync>,
    pub parse_job_description: Arc<dyn Fn(llmuxer::LlmConfig, &str) -> Result<JobDescription, String> + Send + Sync>,
    pub notify_evaluation: Arc<dyn Fn(u32, &mut NotifyData, &Evaluation) -> anyhow::Result<()> + Send + Sync>,
}

impl Default for AppServiceClosures {
    fn default() -> Self {
        Self {
            get_comments_from_url: Arc::new(|_, _| Default::default()),
            evaluate_comment_cached: Arc::new(|_, _, _| Ok(Evaluation::blank())),
            create_evaluation_cache: Arc::new(|_, _, _, _| Ok(String::new())),
            parse_job_description: Arc::new(|_, _| Ok(JobDescription::blank())),
            notify_evaluation: Arc::new(|_, _, _| Ok(())),
        }
    }
}
impl AppService for AppServiceClosures {
    fn get_comments_from_url(&self, url: &str, force: bool) -> async_res!(Vec<Comment>) {
        let url_owned = url.to_owned().clone();
        Box::pin(async move { (self.get_comments_from_url)(&url_owned, force) })
    }

    fn evaluate_comment_cached(
        &self,
        comment: &'_ Comment,
        ev_cache: &'_ EvaluationCache,
        api_key: &'_ str,
    ) -> async_res!('_, anyhow::Result<Evaluation>) {
        let comment = comment.clone();
        let ev_cache = ev_cache.clone();
        let api_key = api_key.to_owned().clone();
        Box::pin(async move { (self.evaluate_comment_cached)(&comment, &ev_cache, &api_key) })
    }

    fn create_evaluation_cache<'a>(
        &self,
        api_key: &'a str,
        pdf_path: &'a Path,
        requirements: &'a str,
        ttl: Duration,
    ) -> async_res!(Result<String, String>) {
        let api_key = api_key.to_owned();
        let pdf_path = pdf_path.to_owned();
        let requirements = requirements.to_owned();
        Box::pin(async move { (self.create_evaluation_cache)(&api_key, &pdf_path, &requirements, ttl) })
    }
    fn parse_job_description(&self, llm_config: llmuxer::LlmConfig, input: &str) -> Result<JobDescription, String> {
        (self.parse_job_description)(llm_config, input)
    }

    fn notify_evaluation(
        &self,
        id: u32,
        notify_data: &mut crate::backend::notify::NotifyData,
        evaluation: &Evaluation,
    ) -> anyhow::Result<()> {
        (self.notify_evaluation)(id, notify_data, evaluation)
    }
}
