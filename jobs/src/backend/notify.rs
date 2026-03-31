use std::collections::HashSet;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::backend::evaluation::Evaluation;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NotifyData {
    pub notified_ids: HashSet<u32>,
    pub threshold: u32,
    pub topic: String,
}
impl Default for NotifyData {
    fn default() -> Self {
        Self {
            notified_ids: HashSet::new(),
            threshold: 75,
            topic: uuid::Uuid::new_v4().to_string(),
        }
    }
}

impl NotifyData {
    #[tracing::instrument(skip(self))]
    pub fn mark_notified(&mut self, comment_id: u32) {
        self.notified_ids.insert(comment_id);
    }
    #[tracing::instrument(skip(eval))]
    pub fn notify_evaluation(&mut self, comment_id: u32, eval: &Evaluation) -> anyhow::Result<()> {
        use anyhow::Context;

        if self.notified_ids.contains(&comment_id) {
            tracing::debug!("Already notified: {}", comment_id);
            return Ok(());
        }
        if eval.score < self.threshold {
            tracing::debug!("Below threshold: {} (score: {})", comment_id, eval.score);
            self.notified_ids.insert(comment_id);
            return Ok(());
        }
        tracing::debug!("Will notify: {}", comment_id);
        let technologies = eval
            .job_description
            .clone()
            .map(|jd| jd.technologies)
            .map(|t| t.join(", "))
            .unwrap_or(String::from("UNKNOWN"));

        let message = format!(
            "Evaluation Score: {}\n\nTechnologies: {}, Eval: {}\nTech: {}\nComp: {}",
            eval.score, technologies, eval.evaluation, eval.technology_alignment, eval.compensation_alignment
        );

        let company_name = eval
            .job_description
            .clone()
            .map(|jd| jd.company_name)
            .unwrap_or(String::from("Unknown company"));

        let topic = self.topic.clone();
        let title = format!("New Job {}", company_name);
        tracing::debug!("Before send spawn");
        tokio::task::spawn(ntfy_notify(topic, title, message));

        self.notified_ids.insert(comment_id);

        Ok(())
    }
    pub fn notified(&self, id: u32) -> bool {
        self.notified_ids.contains(&id)
    }
}

pub async fn ntfy_notify(topic: String, title: String, message: String) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let url = format!("https://ntfy.sh/{}", topic);

    let result = client
        .post(url)
        .header("Title", title)
        .body(message)
        .send()
        .await
        .context("Failed to send ntfy notification");
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::error!("{}", e);
            Err(e)
        }
    }
}
