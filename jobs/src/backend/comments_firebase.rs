use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::time::sleep;

use crate::{backend::comments::clean_html, models::Comment};

const CACHE_FILE: &str = "hn_comments_cache.json";
const EDIT_WINDOW_MINUTES: i64 = 125;

#[derive(Deserialize, Debug, Serialize, Clone)]
struct HnItem {
    id: u32,
    #[serde(default)]
    by: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    time: i64,
    #[serde(default)]
    kids: Vec<u32>,
    #[serde(default)]
    deleted: bool,
    #[serde(default)]
    dead: bool,
}

pub async fn get_comments(item_id: u32, _force: bool) -> Vec<Comment> {
    // 1. Load existing cache
    let mut cache: HashMap<u32, Comment> = fs::read_to_string(CACHE_FILE)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default();

    let semaphore = Arc::new(Semaphore::new(20));
    let client = reqwest::Client::new();

    // 2. Fetch the root item to get the latest list of kids
    let root_item = fetch_hn_item(&client, item_id, Arc::clone(&semaphore)).await;

    let mut results = Vec::new();
    if let Some(root) = root_item {
        let now = Utc::now().timestamp();
        let mut tasks = Vec::new();

        for kid_id in root.kids {
            // Check if we should fetch or use cache
            let should_fetch = if let Some(cached_comment) = cache.get(&kid_id) {
                let age_seconds = now - cached_comment.created_at.timestamp();
                // Re-fetch if it's within the edit window (125 mins)
                age_seconds < (EDIT_WINDOW_MINUTES * 60)
            } else {
                // Not in cache, must fetch
                true
            };
            if should_fetch {
                let client = client.clone();
                let sem = Arc::clone(&semaphore);
                tasks.push(tokio::spawn(async move {
                    fetch_single_comment(&client, kid_id, item_id, sem).await
                }));
            } else {
                // Use cached version
                if let Some(comment) = cache.get(&kid_id) {
                    results.push(comment.clone());
                }
            }
        }

        // 3. Join all new fetch tasks
        let fetched_comments = futures::future::join_all(tasks)
            .await
            .into_iter()
            .flatten() // Flatten JoinHandle result
            .flatten(); // Flatten Option<Comment>

        for comment in fetched_comments {
            cache.insert(comment.id, comment.clone());
            results.push(comment);
        }

        // 4. Update the single cache file
        if let Ok(data) = serde_json::to_string(&cache) {
            let _ = fs::write(CACHE_FILE, data);
        }
    }

    // Return only the comments belonging to this specific item_id
    results.sort_by_key(|c| c.id);
    results
}

async fn fetch_single_comment(
    client: &reqwest::Client,
    id: u32,
    parent_id: u32,
    semaphore: Arc<Semaphore>,
) -> Option<Comment> {
    let item = fetch_hn_item(client, id, semaphore).await?;

    if item.deleted || item.dead {
        return None;
    }

    Some(Comment {
        id: item.id,
        author: item.by,
        text: item.text.as_deref().map(clean_html),
        parent: parent_id,
        created_at: DateTime::from_timestamp(item.time, 0).unwrap_or_else(Utc::now),
        children: item.kids,
    })
}

async fn fetch_hn_item(client: &reqwest::Client, id: u32, semaphore: Arc<Semaphore>) -> Option<HnItem> {
    let url = format!("https://hacker-news.firebaseio.com/v0/item/{}.json", id);
    let mut backoff = Duration::from_millis(500);
    let max_backoff = Duration::from_secs(30);

    for _ in 0..5 {
        let _permit = semaphore.acquire().await.unwrap();
        let response = client.get(&url).send().await;

        match response {
            Ok(res) => {
                if res.status() == StatusCode::TOO_MANY_REQUESTS {
                    let retry_after = res
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|h| h.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(Duration::from_secs)
                        .unwrap_or(backoff);

                    drop(_permit);
                    sleep(retry_after).await;
                    backoff = std::cmp::min(backoff * 2, max_backoff);
                    continue;
                }

                if let Ok(item) = res.json::<HnItem>().await {
                    return Some(item);
                }
                return None;
            }
            Err(_) => {
                drop(_permit);
                sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, max_backoff);
            }
        }
    }
    None
}
