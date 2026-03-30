use std::path::Path;
use std::time::Duration;

use crate::backend::comments::{Comment, get_comments_from_url as comments_get_comments_from_url};
use crate::backend::evaluation::{
    Evaluation, EvaluationCache, create_evaluation_cache as eval_create_evaluation_cache,
    evaluate_comment_cached as eval_evaluate_comment_cached,
};
use crate::backend::job_description::{JobDescription, parse_job_description as jd_parse_job_description};
use crate::backend::notify::NotifyData;
use chrono::Utc;

#[macro_export]
macro_rules! async_res {
    ($out:ty) => {
        std::pin::Pin<Box<dyn std::future::Future<Output = $out> + Send + '_>>
    };
}
pub use async_res;

#[allow(unused)]
pub trait Blank {
    fn blank() -> Self;
}

impl Blank for Evaluation {
    fn blank() -> Self {
        Evaluation {
            evaluation: String::new(),
            technology_alignment: String::new(),
            compensation_alignment: String::new(),
            score: 0,
            job_description: None,
        }
    }
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

pub trait AppService: Send + Sync {
    fn get_comments_from_url(&self, url: &str, force: bool) -> async_res!(Vec<Comment>);
    fn evaluate_comment_cached(
        &self,
        comment: &Comment,
        ev_cache: &EvaluationCache,
        api_key: &str,
    ) -> async_res!(anyhow::Result<Evaluation>);
    fn create_evaluation_cache(
        &self,
        api_key: &str,
        pdf_path: &Path,
        requirements: &str,
        ttl: Duration,
    ) -> async_res!(Result<String, String>);
    fn parse_job_description(&self, llm_config: llmuxer::LlmConfig, input: &str) -> Result<JobDescription, String>;
    fn notify_evaluation(&self, id: u32, notify_data: &mut NotifyData, evaluation: &Evaluation) -> anyhow::Result<()>;
}

impl std::fmt::Debug for dyn AppService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppService")
    }
}
pub struct AppServiceDefault;

impl AppService for AppServiceDefault {
    fn get_comments_from_url(&self, url: &str, force: bool) -> async_res!(Vec<Comment>) {
        let url = url.to_string();
        Box::pin(async move { comments_get_comments_from_url(&url, force).await })
    }

    fn evaluate_comment_cached(
        &self,
        comment: &Comment,
        ev_cache: &EvaluationCache,
        api_key: &str,
    ) -> async_res!(anyhow::Result<Evaluation>) {
        let comment = comment.clone();
        let ev_cache = ev_cache.clone();
        let api_key = api_key.to_string();
        Box::pin(async move { eval_evaluate_comment_cached(&comment, &ev_cache, &api_key).await })
    }

    fn create_evaluation_cache(
        &self,
        api_key: &str,
        pdf_path: &Path,
        requirements: &str,
        ttl: Duration,
    ) -> async_res!(Result<String, String>) {
        let api_key = api_key.to_string();
        let pdf_path = pdf_path.to_path_buf();
        let requirements = requirements.to_string();
        Box::pin(async move { eval_create_evaluation_cache(&api_key, &pdf_path, &requirements, ttl).await })
    }

    fn parse_job_description(&self, llm_config: llmuxer::LlmConfig, input: &str) -> Result<JobDescription, String> {
        jd_parse_job_description(llm_config, input)
    }

    fn notify_evaluation(&self, id: u32, notify_data: &mut NotifyData, evaluation: &Evaluation) -> anyhow::Result<()> {
        notify_data.notify_evaluation(id, evaluation)
    }
}
