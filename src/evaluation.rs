use anyhow::Context;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;
use std::{fs, path::Path};
use tokenizers::Tokenizer;

use crate::comments::Comment;
use crate::job_description::JobDescription;

const MODEL: &str = "gemini-3.1-flash-lite-preview";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Evaluation {
    pub evaluation: String,
    pub technology_alignment: String,
    pub compensation_alignment: String,
    pub score: u32,
    pub job_description: Option<JobDescription>,
}
impl Evaluation {
    pub fn update_job_description(&mut self, jd: JobDescription) {
        self.job_description = Some(jd);
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct EvalCache {
    pub comment_id: u32,
    pub pdf_hash: String,
    pub req_hash: String,
    pub result: Evaluation,
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

pub async fn evaluate_comment(
    comment: &Comment,
    pdf_path_opt: Option<&Path>,
    requirements: &str,
    api_key: &str,
) -> Evaluation {
    let pdf_hash = if let Some(pdf_path) = pdf_path_opt {
        let pdf_data = fs::read(pdf_path).unwrap();
        format!("{:x}", md5::compute(&pdf_data))
    } else {
        String::from("nopdf")
    };
    let req_hash = format!("{:x}", md5::compute(requirements));
    let cache_file = format!("eval_{}_{}.json", comment.id, pdf_hash);

    if let Ok(data) = fs::read_to_string(&cache_file) {
        let cached: EvalCache = serde_json::from_str(&data).unwrap();
        if cached.req_hash == req_hash {
            return cached.result;
        }
    }

    let client = reqwest::Client::new();
    let prompt = format!(
        "Requirements: {}\n\nComment: {}\n\nEvaluate alignment. Return JSON: {{'evaluation': string, 'technology_alignment': string, 'compensation_alignment': string, 'score': 0-100}}",
        requirements,
        comment.text.as_deref().unwrap_or("")
    );

    #[allow(deprecated)]
    let parts = if let Some(pdf_path) = pdf_path_opt {
        let pdf_data = fs::read(pdf_path).unwrap();
        serde_json::json!({
                "inline_data": { "mime_type": "application/pdf", "data": base64::encode(pdf_data) } ,
                "text": prompt
        })
    } else {
        serde_json::json!({
                 "text": prompt
        })
    };
    #[allow(deprecated)]
    let mut payload = serde_json::json!({
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
    payload["contents"] = serde_json::json!({
        "parts": [parts]
    });
    dbg!(&payload);

    let url = format!(
        "{}{}{}{}",
        "https://generativelanguage.googleapis.com/v1beta/models/",
        MODEL,
        ":generateContent?key=",
        api_key
    );

    let raw_res = client.post(url).json(&payload).send().await.unwrap();
    dbg!(&raw_res);
    let res: serde_json::Value = raw_res.json().await.unwrap();
    let raw_json = res["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap();
    dbg!(&raw_json);
    let result: Evaluation = serde_json::from_str(raw_json).unwrap();

    let cache = EvalCache {
        comment_id: comment.id,
        pdf_hash,
        req_hash,
        result: result.clone(),
    };
    fs::write(cache_file, serde_json::to_string(&cache).unwrap()).unwrap();

    result
}
static COMPRESSED_TOKENIZER: &[u8] = include_bytes!("../assets/tokenizer.json.zst");
static TOKENIZER: OnceLock<Tokenizer> = OnceLock::new();
pub fn estimate_accurate_tokens(text: &str) -> usize {
    let tok = TOKENIZER.get_or_init(|| get_tokenizer());

    tok.encode(text, true).map(|e| e.len()).unwrap_or(0)
}
fn get_tokenizer() -> Tokenizer {
    // Decompress zstd blob into a vector
    let mut decoder = zstd::Decoder::new(COMPRESSED_TOKENIZER).unwrap();
    let mut json_bytes = Vec::new();
    decoder.read_to_end(&mut json_bytes).unwrap();

    // Load tokenizer from the JSON buffer in memory
    Tokenizer::from_bytes(json_bytes).expect("Failed to load tokenizer")
}
#[cfg(all(test, feature = "integration-tests"))]
mod tests {

    use super::*;
    use std::env;

    #[tokio::test]
    async fn test_evaluate_comment_integration() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .unwrap();

        let api_key = env::var("TEST_GOOGLE_API_KEY")
            .expect("TEST_GOOGLE_API_KEY must be set for integration tests");

        let comment = Comment {
            id: 123,
            text: Some(
                "I have 5 years of Rust experience and I am looking for competitive compensation."
                    .to_string(),
            ),
            created_at: chrono::Utc::now(),
            author: String::from("test_author"),
            parent: 1,
            children: Vec::new(),
        };

        let requirements = "Candidate must have Rust experience and salary expectation under 150k.";

        let result = evaluate_comment(&comment, None, requirements, &api_key).await;

        assert!(result.score <= 100);
        assert!(!result.evaluation.is_empty());
        println!("Result: {:?}", result);
    }
}
