use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fs, time::Duration};
use url::Url;

#[derive(PartialEq, Eq, Serialize, Deserialize, Debug, Clone)]
pub struct Comment {
    pub created_at: DateTime<Utc>,
    pub id: u32,
    pub author: String,
    pub text: Option<String>,
    pub parent: u32,
    #[serde(default)]
    pub children: Vec<u32>,
}

const MAX_CACHE_AGE: Duration = Duration::from_secs(15 * 60);

pub(in crate::backend) async fn get_comments(item_id: u32, force: bool) -> Vec<Comment> {
    let cache_path = format!("cache_{}.json", item_id);
    if !force {
        if let Ok(metadata) = fs::metadata(&cache_path) {
            let elapsed = metadata
                .modified()
                .unwrap()
                .elapsed()
                .unwrap_or(MAX_CACHE_AGE);
            if elapsed < MAX_CACHE_AGE {
                let data = fs::read_to_string(&cache_path).unwrap();
                return serde_json::from_str(&data).unwrap();
            }
        }
        if let Ok(data) = fs::read_to_string(&cache_path) {
            return serde_json::from_str(&data).unwrap();
        }
    }

    let url = format!("https://hn.algolia.com/api/v1/items/{}", item_id);
    let response: serde_json::Value = reqwest::get(url).await.unwrap().json().await.unwrap();

    let mut comments = Vec::new();
    flatten_comments(&response["children"], item_id, &mut comments);

    fs::write(cache_path, serde_json::to_string(&comments).unwrap()).unwrap();
    comments
}
pub(in crate::backend) async fn get_comments_from_url(url: &str, force: bool) -> Vec<Comment> {
    let item_id = parse_item_id(&url);
    let comments = get_comments(item_id, force).await;
    let comments = filter_top_level(&comments, item_id);
    let comments: Vec<Comment> = comments.into_iter().cloned().collect();
    comments
}

pub(in crate::backend) fn flatten_comments(
    node: &serde_json::Value,
    parent_id: u32,
    acc: &mut Vec<Comment>,
) {
    if let Some(children) = node.as_array() {
        for child in children {
            let id = child["id"].as_u64().unwrap() as u32;
            let date_str = child["created_at"].as_str().unwrap_or("");
            let created_at = date_str
                .parse::<DateTime<Utc>>()
                .unwrap_or_else(|_| Utc::now());
            acc.push(Comment {
                id,
                author: child["author"].as_str().unwrap_or("deleted").to_string(),
                text: child["text"].as_str().map(clean_html).map(String::from),
                parent: parent_id,
                created_at,
                children: child["children"]
                    .as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .map(|c| c["id"].as_u64().unwrap() as u32)
                    .collect(),
            });
            flatten_comments(&child["children"], id, acc);
        }
    }
}
pub(in crate::backend) fn clean_html(html: &str) -> String {
    let fragment = scraper::Html::parse_fragment(html);

    fragment
        .root_element()
        .children()
        .filter_map(|node| {
            let text = match scraper::ElementRef::wrap(node) {
                Some(el) => el.text().collect::<String>(),
                None => node.value().as_text()?.to_string(),
            };

            let trimmed = text.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
pub(in crate::backend) fn filter_top_level(
    comments: &[Comment],
    thread_root_id: u32,
) -> Vec<&Comment> {
    comments
        .iter()
        .filter(|c| c.parent == thread_root_id)
        .filter(|c| c.text.as_ref().map(|v| v.len() > 120).unwrap_or_default())
        .collect()
}

pub(in crate::backend) fn parse_item_id(url: &str) -> u32 {
    let parsed = Url::parse(url).unwrap();
    parsed
        .query_pairs()
        .find(|(key, _)| key == "id")
        .and_then(|(_, val)| val.parse().ok())
        .unwrap()
}
