use anyhow::Context;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use std::{fs, path::Path};

use crate::backend::comments::Comment;
use crate::backend::job_description::JobDescription;

pub const MODEL: &str = "gemini-3.1-flash-lite-preview";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Evaluation {
    pub evaluation: String,
    pub technology_alignment: String,
    pub compensation_alignment: String,
    pub score: u32,
    pub job_description: Option<JobDescription>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct EvaluationCache {
    pub key: String,
    pub timestamp: chrono::DateTime<Utc>,
    pub ttl: Duration,
}
pub trait Usable {
    fn is_usable(&self) -> bool;
}
impl Usable for Option<EvaluationCache> {
    fn is_usable(&self) -> bool {
        if self.is_none() {
            return false;
        }

        self.as_ref().unwrap().is_usable()
    }
}
impl Usable for EvaluationCache {
    fn is_usable(&self) -> bool {
        let td = Utc::now() - self.timestamp;
        if td.num_seconds() > self.ttl.as_secs() as i64 {
            false
        } else {
            true
        }
    }
}

pub async fn evaluate_comment_cached(
    comment: &Comment,
    ev_cache: &EvaluationCache,
    api_key: &str,
) -> anyhow::Result<Evaluation> {
    let client = reqwest::Client::new();

    let prompt = format!(
        "Comment: {}\n\nEvaluate alignment. Be concise (max 3 sentences) for the evaluation field. Return JSON: {{'evaluation': string, 'technology_alignment': string, 'compensation_alignment': string, 'score': 0-100}}",
        comment.text.as_deref().unwrap_or("")
    );
    let payload = serde_json::json!({
        "cached_content": ev_cache.key,
        "contents": [{
            "parts": [{
                "text": prompt,
            }]
        }],
        "generationConfig": {
            "response_mime_type": "application/json",
            "maxOutputTokens": 512,
            "thinkingConfig": {
                "thinkingBudget": 0
            },
            "response_schema": {
                "type": "object",
                "properties": {
                    "evaluation": { "type": "string" },
                    "technology_alignment": { "type": "string" },
                    "compensation_alignment": { "type": "string" },
                    "score": { "type": "integer" }
                }
            }
        }
    });

    let url = format!(
        "{}{}{}{}",
        "https://generativelanguage.googleapis.com/v1beta/models/",
        MODEL,
        ":generateContent?key=",
        api_key
    );

    let res: serde_json::Value = client
        .post(url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .context("Send failure")?
        .json()
        .await
        .context("JSON unwrap failure")?;
    tracing::debug!("raw_response: {:?}", &res);
    let raw_json = res["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .context("Data unwrap error")?;

    serde_json::from_str(raw_json).context("Deserialization error")
}

pub async fn create_evaluation_cache(
    api_key: &str,
    pdf_path: &Path,
    requirements: &str,
    ttl: Duration,
) -> Result<String, String> {
    let time_pad = 10;
    let client = reqwest::Client::new();
    let pdf_data = fs::read(pdf_path).unwrap();

    #[allow(deprecated)]
    let payload = serde_json::json!({
        "model": format!("models/{}",MODEL),
        "ttl": format!("{}s", ttl.as_secs() + time_pad), // 1 hour
        "contents": [
            {
                "role": "user",
                "parts": [
                    { "inline_data": { "mime_type": "application/pdf", "data": base64::encode(pdf_data) } },
                    { "text": format!("Base Requirements for evaluation: {}", requirements) }
                ]

            }
        ],
    });

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/cachedContents?key={}",
        api_key
    );

    let res: serde_json::Value = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    dbg!(&res);

    if let Some(Some(error_msg)) = res.get("error").map(|e| e.get("message")) {
        return Err(error_msg.to_string());
    } else {
        Ok(res["name"].as_str().unwrap().to_string())
    }
}
