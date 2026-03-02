use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::sync::OnceLock;
use std::{fs, path::Path};
use tokenizers::Tokenizer;

use crate::comments::Comment;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Evaluation {
    pub evaluation: String,
    pub technology_alignment: String,
    pub compensation_alignment: String,
    pub score: u8,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct EvalCache {
    pub comment_id: u32,
    pub pdf_hash: String,
    pub req_hash: String,
    pub result: Evaluation,
}

pub async fn evaluate_comment_cached(
    comment: &Comment,
    cache_name: &str, // Pass the name from Step 1
    api_key: &str,
) -> anyhow::Result<Evaluation> {
    let client = reqwest::Client::new();

    let prompt = format!(
        "Comment: {}\n\nEvaluate alignment. Be concise (max 3 sentences) for the evaluation field. Return JSON: {{'evaluation': string, 'technology_alignment': string, 'compensation_alignment': string, 'score': 0-100}}",
        comment.text.as_deref().unwrap_or("")
    );
    // Notice: We only send the specific comment now.
    let payload = serde_json::json!({
        "cached_content": cache_name,
        "contents": [{
            "parts": [{
                "text": prompt,
            }]
        }],
        "generationConfig": {
            "response_mime_type": "application/json",
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
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-3-flash-preview:generateContent?key={}",
        api_key
    );

    let res: serde_json::Value = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .context("Send failure")?
        .json()
        .await
        .context("JSON unwrap failure")?;
    dbg!(&res);
    let raw_json = res["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .context("Data unwrap error")?;

    //let raw_json = raw_json.strip_prefix("```json").unwrap_or(raw_json);
    //let raw_json = raw_json.strip_suffix("```").unwrap_or(raw_json);

    serde_json::from_str(raw_json).context("Deserialization error")
}

pub async fn create_evaluation_cache(
    api_key: &str,
    pdf_path: &Path,
    requirements: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let pdf_data = fs::read(pdf_path).unwrap();

    #[allow(deprecated)]
    let payload = serde_json::json!({
        "model": "models/gemini-3-flash-preview",
        "ttl": "3600s", // 1 hour
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
        // Returns the cache name, e.g., "cachedContents/12345abcde"
        Ok(res["name"].as_str().unwrap().to_string())
    }
}

pub async fn evaluate_comment(
    comment: &Comment,
    pdf_path: &Path,
    requirements: &str,
    api_key: &str,
) -> Evaluation {
    let pdf_data = fs::read(pdf_path).unwrap();
    let pdf_hash = format!("{:x}", md5::compute(&pdf_data));
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
    let payload = serde_json::json!({
        "contents": [
            {
                "parts": [
                    { "inline_data": { "mime_type": "application/pdf", "data": base64::encode(pdf_data) } },
                    { "text": prompt }
                ]
            }
        ],
        "generationConfig": { "response_mime_type": "application/json" }
    });

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-3-flash-preview:generateContent?key={}",
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
