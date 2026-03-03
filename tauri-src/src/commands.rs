use app_core::common_gui::*;
use chrono::Utc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tauri::{Emitter as _, State};
use tauri_plugin_store::StoreExt as _;
use tokio::sync::Semaphore;

use app_core::{
    comments::{self, Comment},
    evaluation::{Evaluation, create_evaluation_cache, evaluate_comment, evaluate_comment_cached},
};

use crate::{AppStateHolder, CommentRow, STATE_KEY, STORE_PATH};

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
pub fn set_sort(app: tauri::AppHandle, state: State<AppStateHolder>, column: SortColumn) {
    {
        let mut s = state.0.write();
        s.descending = !s.descending;
        s.sort_column = column;
    }
    let _ = app.emit("refresh-rows", get_rows(state));
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
    tauri::async_runtime::spawn(async move {
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
    tauri::async_runtime::spawn(async move {
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

pub async fn evaluate_single_comment_sem(
    comment: &Comment,
    app_state: Arc<RwLock<AppState>>,
) -> Result<(), String> {
    let id = comment.id;
    let comment = comment.clone();
    let app_state = app_state.clone();
    let (cache_key, api_key) = {
        let state = app_state.read();
        let api_key = state.api_key.clone();
        let Some(cache_key) = state.eval_cache.clone() else {
            eprintln!("Missing Cache Evaluation Key");
            return Err(String::from("Missing cache evaluation key"));
        };
        (cache_key, api_key)
    };
    let eval = evaluate_comment_cached(&comment, &cache_key, &api_key).await;
    {
        let mut state_w = app_state.write();
        match eval {
            Ok(e) => {
                state_w.evaluations.insert(id, e);
                Ok(())
            }
            Err(err) => Err(err.to_string()),
        }
    }
}
#[tauri::command]
pub fn batch_process(state: State<AppStateHolder>) {
    let state = Arc::clone(&state.0);
    tauri::async_runtime::spawn(async move {
        {
            let mut s = state.write();
            s.processing.enabled = true;
        }
        let semaphore = Arc::new(Semaphore::new(50)); // Max 5 concurrent requests
        let mut handles = vec![];

        let evaluations = state.read().evaluations.clone();
        let comments = state
            .read()
            .comments
            .clone()
            .into_iter()
            .filter(|c| !evaluations.contains_key(&c.id))
            .collect::<Vec<_>>();

        {
            let mut state = state.write();
            state.processing.total = comments.len();
            state.processing.done = 0;
            state.processing.error = 0;
        }

        for comment in comments {
            let sem = Arc::clone(&semaphore);
            let state_c = state.clone();
            let handle = tokio::spawn(async move {
                for _ in 0..3 {
                    let _permit = sem.acquire().await.unwrap();
                    if !state_c.read().processing.enabled {
                        return;
                    }
                    if state_c.read().evaluations.contains_key(&comment.id) {
                        return;
                    };
                    let ev_result = evaluate_single_comment_sem(&comment, state_c.clone()).await;
                    let mut state_w = state_c.write();
                    match ev_result {
                        Ok(_) => {
                            state_w.processing.done += 1;
                            return;
                        }
                        Err(_) => {
                            continue;
                        }
                    };
                }
                state_c.write().processing.error += 1;
            });
            handles.push(handle);
        }

        let _ = futures::future::join_all(handles).await;
        Ok::<(), String>(())
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
    tauri::async_runtime::spawn(async move {
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

#[tauri::command]
pub fn get_rows(state: State<AppStateHolder>) -> Vec<CommentRow> {
    let s = state.0.read();
    let indices: Vec<usize> = (0..s.comments.len()).collect();
    let mut indices: Vec<usize> = indices
        .into_iter()
        .filter(|i| {
            let comment_id = s.comments[*i].id;
            !s.flags
                .get(&comment_id)
                .cloned()
                .unwrap_or_default()
                .get_hide()
        })
        .collect();

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

#[tauri::command]
pub fn save_state(app: tauri::AppHandle, state: State<AppStateHolder>) {
    let s = state.0.read().clone();
    let store = app.store(STORE_PATH).unwrap();
    store.set(STATE_KEY, serde_json::to_value(s).unwrap());
    store.save().unwrap();
}


use paste::paste;
macro_rules! flag_command {
    ($name: ident) => {
        paste!{
            #[tauri::command]
            pub fn [<comment_flag_ $name>](app: tauri::AppHandle, state: State<AppStateHolder>, comment_id: u32, toggle: bool) {
                let mut flags = state
                    .0
                    .read()
                    .flags
                    .get(&comment_id)
                    .cloned()
                    .unwrap_or_default();
                flags.[<set_ $name>](toggle);
                state.0.write().flags.insert(comment_id, flags.clone());
            }
        }
    }
}
flag_command!(hide);
flag_command!(seen);
flag_command!(in_progress);


#[derive(Clone, Serialize, Default)]
pub struct FlagsState {
    hidden: bool,
    in_progress: bool,
    seen: bool,
}

#[tauri::command]
pub fn comment_flags(state: State<AppStateHolder>, comment_id: u32) -> FlagsState {
    let flags = state
        .0
        .read()
        .flags
        .get(&comment_id)
        .cloned()
        .unwrap_or_default();
    FlagsState {
        hidden: flags.get_hide(),
        in_progress: flags.get_in_progress(),
        seen: flags.get_seen(),
    }
}
#[tauri::command]
pub fn open_url(url: String) {
    let _ = std::process::Command::new("open")
        .arg(&url)
        .spawn() // macOS
        .or_else(|_| std::process::Command::new("xdg-open").arg(&url).spawn()) // Linux
        .or_else(|_| {
            std::process::Command::new("cmd")
                .args(["/c", "start", &url])
                .spawn()
        }); // Windows
}
