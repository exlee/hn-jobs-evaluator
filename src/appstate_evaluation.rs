use parking_lot::RwLock;

use crate::comments::Comment;
use crate::common_gui::AppState;
use crate::evaluation;
use std::path::PathBuf;
use std::sync::Arc;

const RETRY_JOB_DESCRIPTION: usize = 3;
const RETRY_EVALUATION: usize = 3;

/// Evaluate a Single Comment
#[tracing::instrument(skip(app_state,comment), fields(comment_id = comment.id))]
pub async fn evaluate_single_comment(
    comment: &Comment,
    app_state: Arc<RwLock<AppState>>,
) -> Result<(), String> {
    let id = comment.id;
    let comment = comment.clone();

    let (cache_key, api_key, job_descriptions) = {
        let state = app_state.read();
        (
            state.eval_cache.clone(),
            state.api_key.clone(),
            state.job_descriptions.clone(),
        )
    };

    let Some(cache_key) = cache_key else {
        return Err(String::from("Missing cache evaluation key"));
    };

    let comment_text = comment.text.clone().unwrap_or_default();
    let api_key_c = api_key.clone();

    tracing::debug!("pre-jd");
    let jd_fut = tokio::task::spawn_blocking(move || {
        let mut jd_container = job_descriptions;
        let mut last_res = Err("Failed to fetch JD".to_string());
        for _ in 0..RETRY_JOB_DESCRIPTION {
            match jd_container.get(id, &comment_text, &api_key_c) {
                Ok(jd) => return (Ok(jd), jd_container),
                Err(e) => last_res = Err(e.to_string()),
            }
        }
        (last_res, jd_container)
    });

    tracing::debug!("pre-eval");
    let eval_fut = async {
        let mut last_res = Err(anyhow::anyhow!("Failed to evaluate"));
        for _ in 0..RETRY_EVALUATION {
            match evaluation::evaluate_comment_cached(&comment, &cache_key, &api_key).await {
                Ok(e) => return Ok(e),
                Err(e) => last_res = Err(e),
            }
        }
        last_res
    };

    tracing::debug!("pre-join");
    let (jd_tuple, eval_res) = tokio::join!(jd_fut, eval_fut);
    tracing::debug!("post-join");
    let (jd_result, job_descriptions) = jd_tuple.map_err(|e| e.to_string())?;

    app_state.write().job_descriptions = job_descriptions;

    match eval_res {
        Ok(mut e) => {
            match jd_result {
                Ok(jd) => {
                    tracing::debug!("Got JD: {:?}", jd);
                    e.update_job_description(jd);
                }
                Err(e) => {
                    tracing::error!("{}", e);
                }
            }
            let _ = app_state.write().notify_data.notify_evaluation(id, &e);
            app_state.write().evaluations.insert(id, e);
            app_state.write().processing.done += 1;
            Ok(())
        }
        Err(err) => {
            app_state.write().processing.error += 1;
            Err(err.to_string())
        }
    }
}
#[allow(unused)]
fn evaluate_single_comment_live(comment: &Comment, app_state: Arc<RwLock<AppState>>) {
    let id = comment.id;
    let comment = comment.clone();
    let app_state = app_state.clone();
    tokio::spawn(async move {
        let (requirements, api_key, pathbuf) = {
            let state = app_state.read();
            let Some(pdf_path) = state.pdf_path.clone() else {
                eprintln!("Missing PDF Path");
                return;
            };
            let requirements = state.requirements.clone();
            let api_key = state.api_key.clone();
            let pathbuf = PathBuf::from(pdf_path);
            (requirements, api_key, pathbuf)
        };
        let eval =
            evaluation::evaluate_comment(&comment, Some(&pathbuf), &requirements, &api_key).await;
        app_state.write().evaluations.insert(id, eval);
    });
}
