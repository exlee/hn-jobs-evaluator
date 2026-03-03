use crate::common_gui::*;
use chrono::Utc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tauri::State;
use tauri_plugin_store::StoreExt as _;

use crate::{
    comments::{self, Comment},
    evaluation::{Evaluation, create_evaluation_cache, evaluate_comment, evaluate_comment_cached},
};

pub struct AppStateHolder(pub Arc<RwLock<AppState>>);

// --- Commands ---

#[tauri::command]
pub fn get_state(state: State<AppStateHolder>) -> AppState {
    state.0.read().clone()
}

#[tauri::command]
pub fn update_field(state: State<AppStateHolder>, field: String, value: String) {
    let mut s = state.0.write();
    match field.as_str() {
        "hn_url" => s.hn_url = value,
        "api_key" => s.api_key = value,
        "requirements" => s.requirements = value,
        "pdf_path" => s.pdf_path = Some(value),
        _ => {}
    }
}

#[tauri::command]
pub fn set_sort(state: State<AppStateHolder>, column: SortColumn) {
    let mut s = state.0.write();
    s.descending = !s.descending;
    s.sort_column = column;
}

#[tauri::command]
pub fn nuke_evaluations(state: State<AppStateHolder>) {
    state.0.write().evaluations = HashMap::new();
}

#[tauri::command]
pub fn select_pdf(state: State<AppStateHolder>) -> Option<String> {
    let path = rfd::FileDialog::new()
        .add_filter("PDF", &["pdf"])
        .pick_file()?;
    let path_str = path.display().to_string();
    state.0.write().pdf_path = Some(path_str.clone());
    Some(path_str)
}

#[tauri::command]
pub fn get_evaluation_cache(state: State<AppStateHolder>) {
    let state = Arc::clone(&state.0);
    tokio::spawn(async move {
        let (api_key, pdf_path, requirements) = {
            let s = state.read();
            (
                s.api_key.clone(),
                PathBuf::from(s.pdf_path.clone().unwrap()),
                s.requirements.clone(),
            )
        };
        let ttl = Duration::from_secs(3600);

        match create_evaluation_cache(&api_key, &pdf_path, &requirements, ttl).await {
            Ok(cache_key) => {
                let ev_cache = EvaluationCache {
                    key: cache_key,
                    timestamp: Utc::now(),
                    ttl: ttl,
                };
                state.write().eval_cache = Some(ev_cache);
            }
            Err(err) => state.write().cache_key_error = Some(err),
        }
    });
}

#[tauri::command]
pub fn process_comments(state: State<AppStateHolder>, force: bool) {
    let state = Arc::clone(&state.0);
    tokio::spawn(async move {
        state.write().comments = vec![];
        let hn_url = state.read().hn_url.clone();
        let item_id = comments::parse_item_id(&hn_url);
        let fetched = comments::get_comments(item_id, force).await;
        let top_level: Vec<Comment> = comments::filter_top_level(&fetched, item_id)
            .into_iter()
            .cloned()
            .collect();
        state.write().comments = top_level;
    });
}

#[tauri::command]
pub fn batch_process(state: State<AppStateHolder>) {
    let state = Arc::clone(&state.0);
    tokio::spawn(async move {
        {
            let mut s = state.write();
            s.processing.enabled = true;
        }
        // ... same batch logic as before ...
    });
}

#[tauri::command]
pub fn evaluate_comment_cmd(state: State<AppStateHolder>, comment_id: u32) {
    let state = Arc::clone(&state.0);
    let comment = state
        .read()
        .comments
        .iter()
        .find(|c| c.id == comment_id)
        .cloned()
        .unwrap();
    tokio::spawn(async move {
        let (ev_cache, api_key) = {
            let s = state.read();
            (s.eval_cache.clone().unwrap(), s.api_key.clone())
        };
        let eval = evaluate_comment_cached(&comment, &ev_cache, &api_key).await;
        {
            let mut state_w = state.write();
            match eval {
                Ok(e) => {
                    state_w.evaluations.insert(comment.id, e);
                    Ok(())
                }
                Err(err) => Err(err.to_string()),
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppStateHolder(Arc::new(RwLock::new(AppState::default()))))
        .invoke_handler(tauri::generate_handler![
            get_state,
            update_field,
            set_sort,
            nuke_evaluations,
            select_pdf,
            get_evaluation_cache,
            process_comments,
            batch_process,
            evaluate_comment_cmd
        ])
        .run(tauri::generate_context!())

        .expect("error while running tauri application");
}

#[derive(Serialize, Clone)]
pub struct CommentRow {
    pub comment: Comment,
    pub eval: Option<Evaluation>,
}

#[tauri::command]
pub fn get_rows(state: State<AppStateHolder>) -> Vec<CommentRow> {
    let s = state.0.read();
    let mut indices: Vec<usize> = (0..s.comments.len()).collect();

    indices.sort_by(|&a, &b| {
        let res = match s.sort_column {
            SortColumn::Score => {
                let sa = s
                    .evaluations
                    .get(&s.comments[a].id)
                    .map(|e| e.score)
                    .unwrap_or(0);
                let sb = s
                    .evaluations
                    .get(&s.comments[b].id)
                    .map(|e| e.score)
                    .unwrap_or(0);
                sa.cmp(&sb)
            }
            SortColumn::Id => s.comments[a].id.cmp(&s.comments[b].id),
            SortColumn::CreatedAt => s.comments[a].created_at.cmp(&s.comments[b].created_at),
        };
        if s.descending { res.reverse() } else { res }
    });

    indices
        .iter()
        .map(|&i| CommentRow {
            eval: s.evaluations.get(&s.comments[i].id).cloned(),
            comment: s.comments[i].clone(),
        })
        .collect()
}

const STORE_PATH: &str = "app_state.json";
const STATE_KEY: &str = "state";

#[tauri::command]
pub fn save_state(app: tauri::AppHandle, state: State<AppStateHolder>) {
    let s = state.0.read().clone();
    let store = app.store(STORE_PATH).unwrap();
    store.set(STATE_KEY, serde_json::to_value(s).unwrap());
    store.save().unwrap();
}

// Load on startup — call this in run() before .manage()
pub fn load_state(app: &tauri::AppHandle) -> AppState {
    let store = app.store(STORE_PATH).unwrap();
    store
        .get(STATE_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}
