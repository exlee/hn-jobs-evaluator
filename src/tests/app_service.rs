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
}

pub struct AppServiceClosures {
    pub get_comments_from_url: Box<dyn Fn(&str, bool) -> async_res!(Vec<Comment>) + Send + Sync>,
    pub evaluate_comment_cached:
        Box<dyn Fn(&Comment, &EvaluationCache, &str) -> async_res!(anyhow::Result<Evaluation>) + Send + Sync>,
    pub create_evaluation_cache:
        Box<dyn Fn(&str, &Path, &str, Duration) -> async_res!(Result<String, String>) + Send + Sync>,
    pub parse_job_description: Box<dyn Fn(llmuxer::LlmConfig, &str) -> Result<JobDescription, String> + Send + Sync>,
}

impl AppService for AppServiceClosures {
    fn get_comments_from_url(&self, url: &str, force: bool) -> async_res!(Vec<Comment>) {
        (self.get_comments_from_url)(url, force)
    }

    fn evaluate_comment_cached(
        &self,
        comment: &Comment,
        ev_cache: &EvaluationCache,
        api_key: &str,
    ) -> async_res!(anyhow::Result<Evaluation>) {
        (self.evaluate_comment_cached)(comment, ev_cache, api_key)
    }

    fn create_evaluation_cache(
        &self,
        api_key: &str,
        pdf_path: &Path,
        requirements: &str,
        ttl: Duration,
    ) -> async_res!(Result<String, String>) {
        (self.create_evaluation_cache)(api_key, pdf_path, requirements, ttl)
    }

    fn parse_job_description(&self, llm_config: llmuxer::LlmConfig, input: &str) -> Result<JobDescription, String> {
        (self.parse_job_description)(llm_config, input)
    }
}
