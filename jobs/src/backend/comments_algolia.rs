use std::fs;

use crate::{
    backend::comments::{MAX_CACHE_AGE, flatten_comments},
    models::Comment,
};
pub(in crate::backend) async fn get_comments(item_id: u32, force: bool) -> Vec<Comment> {
    let cache_path = format!("cache_{}.json", item_id);
    if !force {
        if let Ok(metadata) = fs::metadata(&cache_path) {
            let elapsed = metadata.modified().unwrap().elapsed().unwrap_or(MAX_CACHE_AGE);
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
