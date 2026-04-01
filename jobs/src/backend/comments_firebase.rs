use std::fs;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::Deserialize;
use tokio::sync::Semaphore;
use tokio::time::sleep;

use crate::{
    backend::comments::{MAX_CACHE_AGE, clean_html},
    models::Comment,
};

#[derive(Deserialize, Debug)]
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

pub async fn get_comments(item_id: u32, force: bool) -> Vec<Comment> {
    let cache_path = format!("cache_{}.json", item_id);
    if !force {
        if let Ok(metadata) = fs::metadata(&cache_path) {
            let elapsed = metadata.modified().unwrap().elapsed().unwrap_or(MAX_CACHE_AGE);
            if elapsed < MAX_CACHE_AGE {
                if let Ok(data) = fs::read_to_string(&cache_path) {
                    if let Ok(comments) = serde_json::from_str::<Vec<Comment>>(&data) {
                        return comments;
                    }
                }
            }
        }
    }

    let semaphore = Arc::new(Semaphore::new(20));
    let client = reqwest::Client::new();

    let root_item = fetch_hn_item(&client, item_id, Arc::clone(&semaphore)).await;
    let mut all_comments = Vec::new();

    if let Some(root) = root_item {
        let mut tasks = Vec::new();
        for kid_id in root.kids {
            let client = client.clone();
            let sem = Arc::clone(&semaphore);
            tasks.push(tokio::spawn(async move {
                fetch_single_comment(&client, kid_id, item_id, sem).await
            }));
        }

        let results = futures::future::join_all(tasks).await.into_iter().flatten().flatten();
        for comment in results {
            all_comments.push(comment);
        }
    }

    let _ = fs::write(&cache_path, serde_json::to_string(&all_comments).unwrap());
    all_comments
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
        children: item.kids, // We still keep the IDs of children if the model requires it, but we don't fetch them
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
