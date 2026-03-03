use app_core::common_gui::*;
use chrono::Utc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tauri::{Manager as _, State};
use tauri_plugin_store::StoreExt as _;

use app_core::{
    comments::{self, Comment},
    evaluation::{Evaluation, create_evaluation_cache, evaluate_comment, evaluate_comment_cached},
};

mod commands;

pub struct AppStateHolder(pub Arc<RwLock<AppState>>);

// --- Commands ---

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .setup(|app| {
            let state = load_state(&app.handle());
            app.manage(AppStateHolder(Arc::new(RwLock::new(state))));
            Ok(())
        })
        //.manage(AppStateHolder(Arc::new(RwLock::new(AppState::default()))))
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::update_field,
            commands::set_sort,
            commands::nuke_evaluations,
            commands::select_pdf,
            commands::get_evaluation_cache,
            commands::process_comments,
            commands::batch_process,
            commands::evaluate_comment_cmd,
            commands::get_rows,
            commands::save_state,
            commands::comment_flag_hide,
            commands::comment_flag_seen,
            commands::comment_flag_progress,
            commands::comment_flags,
            commands::open_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[derive(Serialize, Clone)]
pub struct CommentRow {
    pub comment: Comment,
    pub eval: Option<Evaluation>,
}

const STORE_PATH: &str = "app_state.json";
const STATE_KEY: &str = "state";
// Load on startup — call this in run() before .manage()
pub fn load_state(app: &tauri::AppHandle) -> AppState {
    let store = app.store(STORE_PATH).unwrap();
    let import_path = PathBuf::from("../state.json");
    println!("Loading procedure");
    if import_path.exists() {
        println!("Path exists");
        if let Ok(json_str) = std::fs::read_to_string(import_path) {
            println!("Read json");
            if let Ok(state) = serde_json::from_str::<AppState>(&json_str) {
                println!("Got state");
                return state;
            }
        }
    }
    store
        .get(STATE_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .or_else(|| {
            let toml_string = std::fs::read_to_string("run_spec.toml")
                .map_err(|_| String::from("can't read toml"))
                .ok()?;
            let run_spec: RunSpec = toml::from_str(&toml_string)
                .map_err(|_| String::from("Toml mapping error"))
                .ok()?;
            Some(AppState {
                hn_url: run_spec.hn_url,
                pdf_path: Some(run_spec.pdf_path),
                api_key: run_spec.api_key,
                requirements: run_spec.requirements,
                ..Default::default()
            })
        })
        .unwrap_or_default()
}
