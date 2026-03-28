use crate::{appstate_evaluation, autofetcher, common_gui::*, evaluation};
use chrono::Utc;
use eframe::egui::{self, Button, Color32, Layout, Widget};
use parking_lot::RwLock;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{
    Semaphore,
    mpsc::{Receiver, Sender},
};

use crate::{
    comments::{self, Comment},
    evaluation::{Usable, create_evaluation_cache, estimate_accurate_tokens},
};

struct App {
    state: Arc<RwLock<AppState>>,
    comments_tx: Sender<Vec<Comment>>,
    comments_rx: Receiver<Vec<Comment>>,
}

fn batch_process(comments: &[Comment], state: Arc<RwLock<AppState>>) -> Result<(), String> {
    let mut comments = comments.to_vec();
    comments.sort_by_key(|c| c.created_at);
    tokio::spawn(async move {
        let semaphore = Arc::new(Semaphore::new(50)); // Max 5 concurrent requests
        let mut handles = vec![];

        let evaluations = state.read().evaluations.clone();
        let comments = comments
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
                    let ev_result =
                        appstate_evaluation::evaluate_single_comment(&comment, state_c.clone())
                            .await;
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
    });

    Ok(())
}

fn get_evaluation_cache(state: Arc<RwLock<AppState>>) {
    tokio::spawn(async move {
        {
            let mut state_w = state.write();
            state_w.eval_cache = None;
            state_w.cache_key_error = None;
        }
        let (api_key, pdf_path, requirements) = {
            let state = state.read();
            let pdf_path = state.pdf_path.clone();
            (
                state.api_key.clone(),
                PathBuf::from(pdf_path.unwrap().clone()),
                state.requirements.clone(),
            )
        };
        let ttl = Duration::from_hours(24);
        match create_evaluation_cache(&api_key, &pdf_path, &requirements, ttl).await {
            Ok(cache_key) => {
                let ev_cache = evaluation::EvaluationCache {
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

fn process_comments(state: Arc<RwLock<AppState>>, force: bool) {
    tokio::spawn(async move {
        {
            let mut state = state.write();
            state.comments = vec![];
        }
        let hn_url = state.read().hn_url.clone();
        let top_level = comments::get_comments_from_url(&hn_url, force).await;

        let mut state_w = state.write();
        state_w.comments = top_level.into_iter().map(|t| t.clone()).collect();
    });
}

pub struct SizedButton {
    size: egui::Vec2,
    label: &'static str,
}
impl SizedButton {
    fn new<T>(size: T, label: &'static str) -> Self
    where
        T: Into<egui::Vec2>,
    {
        Self {
            size: size.into(),
            label,
        }
    }
}

impl Widget for SizedButton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let button = Button::new(self.label);
        ui.add_sized(self.size, button)
    }
}
pub trait TableCell {
    //fn table_cell<R>(&mut self, cols: &[f32], col_number: usize, fun: &dyn Fn(&mut egui::Ui) -> R);
    fn table_cell<F>(&mut self, cols: &[f32], avw: f32, col_number: usize, fun: F)
    where
        F: FnOnce(&mut Self);
    fn table_cell_h<F>(&mut self, cols: &[f32], avw: f32, col_number: usize, fun: F)
    where
        F: FnOnce(&mut Self);
}
impl TableCell for egui::Ui {
    fn table_cell<F>(&mut self, cols: &[f32], avw: f32, col_number: usize, fun: F)
    where
        F: FnOnce(&mut egui::Ui),
    {
        let cell_size = egui::vec2(avw * cols[col_number], 200.0);
        self.allocate_ui_with_layout(
            cell_size,
            egui::Layout::top_down(egui::Align::Min).with_cross_justify(true),
            fun,
        );
    }
    fn table_cell_h<F>(&mut self, cols: &[f32], avw: f32, col_number: usize, fun: F)
    where
        F: FnOnce(&mut egui::Ui),
    {
        let cell_size = egui::vec2(avw * cols[col_number], 20.0);

        self.allocate_ui_with_layout(
            cell_size,
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            fun,
        );
    }
}
fn scrollable_row(ui: &mut egui::Ui, id_salt: String, text: &str) {
    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
        egui::ScrollArea::vertical()
            .id_salt(id_salt)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    let mut job = egui::text::LayoutJob::default();

                    job.append(
                        text,
                        0.0,
                        egui::text::TextFormat {
                            font_id: egui::FontId::proportional(12.0),
                            color: ui.visuals().text_color(),
                            line_height: Some(20.0),
                            ..Default::default()
                        },
                    );

                    // Mandatory for ScrollArea to trigger vertical scrolling
                    job.wrap.max_width = ui.available_width();
                    ui.add_space(10.0);
                    ui.add(egui::Label::new(job).wrap());
                    ui.add_space(10.0);
                });
            });
    });
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_everforest_theme(&cc.egui_ctx, cc.egui_ctx.style().visuals.dark_mode);
        let state_rwlock: AppState = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, eframe::APP_KEY))
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
            .unwrap_or_default();

        let state = Arc::new(RwLock::new(state_rwlock.clone()));
        let (comments_tx, comments_rx) = tokio::sync::mpsc::channel(1);

        Self {
            state,
            comments_tx,
            comments_rx,
        }
    }

    fn render_table(&mut self, ui: &mut egui::Ui, _available_w: f32) {
        let mut state = self.state.write();
        let indices: Vec<usize> = (0..state.comments.len()).collect();
        let mut indices: Vec<usize> = indices
            .into_iter()
            .filter(|i| {
                if state.search_string.is_empty() {
                    return true;
                }
                let mut matches = false;
                let comment = state.comments[*i].clone();
                //if let Some(evaluation) = state.evaluations.get(&comment.id) {}
                if let Some(t) = comment.text
                    && t.contains(&state.search_string)
                {
                    matches = true;
                }
                matches
            })
            .filter(|i| {
                if state.min_score == 0 {
                    return true;
                }
                let comment = state.comments[*i].clone();
                let Some(evaluation) = state.evaluations.get(&comment.id) else {
                    return false;
                };
                evaluation.score >= state.min_score
            })
            .filter(|i| {
                if !state.hide_seen {
                    return true;
                }
                let comment = state.comments[*i].clone();
                let Some(flags) = state.flags.get(&comment.id) else {
                    return true;
                };
                if flags.get_seen() { false } else { true }
            })
            .filter(|i| {
                if !state.hide_in_progress {
                    return true;
                }
                let comment = state.comments[*i].clone();
                let Some(flags) = state.flags.get(&comment.id) else {
                    return true;
                };
                if flags.get_in_progress() { false } else { true }
            })
            .filter(|i| {
                let comment_id = state.comments[*i].id;
                !state
                    .flags
                    .get(&comment_id)
                    .cloned()
                    .unwrap_or_default()
                    .get_hide()
            })
            .collect();

        // Sorting logic
        indices.sort_by(|&a, &b| {
            let res = match state.sort_column {
                SortColumn::Score => {
                    let s_a = state
                        .evaluations
                        .get(&state.comments[a].id)
                        .map(|e| e.score)
                        .unwrap_or(0);
                    let s_b = state
                        .evaluations
                        .get(&state.comments[b].id)
                        .map(|e| e.score)
                        .unwrap_or(0);
                    s_a.cmp(&s_b)
                }
                SortColumn::Id => state.comments[a].id.cmp(&state.comments[b].id),
                SortColumn::CreatedAt => state.comments[a]
                    .created_at
                    .cmp(&state.comments[b].created_at),
            };
            if state.descending { res.reverse() } else { res }
        });

        // indices.sort_by_key(|i| {
        //     let comment_id = state.comments[*i].id;
        //     state.flags.entry(comment_id).or_default().clone()
        // });

        let cols = vec![0.1, 0.375, 0.125, 0.125, 0.125, 0.03, 0.12];
        let spacing = ui.spacing().item_spacing.x;
        let available_w = ui.max_rect().width() - spacing * (cols.len() - 1) as f32;

        //let available_w = ui.clip_rect().width();
        egui::Grid::new("header")
            .num_columns(cols.len())
            .striped(true)
            .show(ui, |ui| {
                ui.table_cell_h(&cols, available_w, 0, |ui| {
                    if ui.button("CreatedAt").clicked() {
                        state.sort_column = SortColumn::CreatedAt;
                        state.descending = !state.descending;
                    }
                });

                ui.table_cell_h(&cols, available_w, 1, |ui| {
                    if ui.button("Comment").clicked() {
                        state.sort_column = SortColumn::Id;
                        state.descending = !state.descending;
                    }
                });

                ui.table_cell_h(&cols, available_w, 2, |ui| {
                    ui.label("Evaluation");
                });

                ui.table_cell_h(&cols, available_w, 3, |ui| {
                    ui.label("Tech Alignment");
                });

                ui.table_cell_h(&cols, available_w, 4, |ui| {
                    ui.label("Comp Alignment");
                });

                ui.table_cell_h(&cols, available_w, 5, |ui| {
                    if ui.button("Score").clicked() {
                        state.sort_column = SortColumn::Score;
                        state.descending = !state.descending;
                    }
                });

                ui.table_cell_h(&cols, available_w, 6, |ui| {
                    ui.label(" ");
                });

                ui.end_row();
            });

        ui.separator();

        let available_w = ui.max_rect().width() - spacing * (cols.len() - 1) as f32;
        egui::ScrollArea::vertical()
            .hscroll(false) // CRITICAL: Forces grid to fit window width
            .auto_shrink([false; 2]) // Prevents the area from collapsing if empty
            .show_rows(ui, 200.0, indices.len(), |ui, row_range| {
                egui::Grid::new("body")
                    .num_columns(cols.len())
                    .min_row_height(200.0)
                    .striped(true)
                    .show(ui, |ui| {
                        for row_idx in row_range {
                            let idx = indices[row_idx];
                            let comment = &state.comments[idx].clone();
                            let eval = state.evaluations.get(&comment.id);
                            let flags = state.flags.get(&comment.id).cloned().unwrap_or_default();

                            if flags.get_in_progress() {
                                ui.style_mut().visuals.override_text_color =
                                    Some(egui::Color32::from_rgb(100, 255, 100));
                                //ui.style_mut().visuals.override_text_color = Some(egui::Color32::GREEN);
                            } else if flags.get_seen() {
                                ui.style_mut().visuals.override_text_color =
                                    Some(egui::Color32::from_white_alpha(64));
                            } else {
                                ui.style_mut().visuals.override_text_color = None;
                            }
                            // CreatedAt
                            ui.table_cell(&cols, available_w, 0, |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.label(format!("{}", comment.created_at));
                                });
                            });
                            // Comment
                            ui.table_cell(&cols, available_w, 1, |ui| {
                                ui.vertical(|ui| {
                                    ui.hyperlink_to(
                                        egui::RichText::new(format!("#{}", comment.id)).small(),
                                        format!(
                                            "https://news.ycombinator.com/item?id={}",
                                            comment.id
                                        ),
                                    );
                                    scrollable_row(
                                        ui,
                                        format!("comment_{}", comment.id),
                                        comment.text.as_deref().unwrap_or(""),
                                    );
                                });
                            });
                            // Evaluation
                            ui.table_cell(&cols, available_w, 2, |ui| {
                                if let Some(e) = eval {
                                    scrollable_row(
                                        ui,
                                        format!("eval_{}", comment.id),
                                        &e.evaluation,
                                    );
                                } else {
                                    ui.centered_and_justified(|ui| ui.label("-"));
                                }
                            });

                            // Tech alignment
                            ui.table_cell(&cols, available_w, 3, |ui| {
                                if let Some(e) = eval {
                                    scrollable_row(
                                        ui,
                                        format!("tech_{}", comment.id),
                                        &e.technology_alignment,
                                    );
                                } else {
                                    ui.centered_and_justified(|ui| ui.label("-"));
                                }
                            });

                            // Comp alignment
                            ui.table_cell(&cols, available_w, 4, |ui| {
                                if let Some(e) = eval {
                                    scrollable_row(
                                        ui,
                                        format!("comp_{}", comment.id),
                                        &e.compensation_alignment,
                                    );
                                } else {
                                    ui.centered_and_justified(|ui| ui.label("-"));
                                }
                            });

                            // Score
                            ui.table_cell(&cols, available_w, 5, |ui| {
                                ui.vertical_centered(|ui| {
                                    if let Some(e) = eval {
                                        ui.label(format!("{}", e.score));
                                    } else {
                                        ui.centered_and_justified(|ui| ui.label("-"));
                                    }
                                });
                            });

                            ui.table_cell(&cols, available_w, 6, |ui| {
                                let button_size = egui::vec2(
                                    available_w * cols[6] * 0.6,
                                    ui.spacing().interact_size.y,
                                );
                                let button = SizedButton::new(button_size, "Evaluate");
                                let resp = ui.add_enabled(state.eval_cache.is_usable(), button);

                                if resp.clicked() {
                                    let handle = self.state.clone();
                                    let comment = comment.clone();
                                    tokio::task::spawn(async move {
                                        let _ = appstate_evaluation::evaluate_single_comment(
                                            &comment, handle,
                                        )
                                        .await;
                                    });
                                };
                                let mut flags =
                                    state.flags.get(&comment.id).cloned().unwrap_or_default();

                                if flags.get_hide() {
                                    let resp = ui.add(SizedButton::new(button_size, "Unhide"));
                                    if resp.clicked() {
                                        flags.set_hide(false);
                                        state.flags.insert(comment.id.clone(), flags.clone());
                                    }
                                } else {
                                    let resp = ui.add(SizedButton::new(button_size, "Hide"));
                                    if resp.clicked() {
                                        flags.set_hide(true);
                                        state.flags.insert(comment.id.clone(), flags.clone());
                                    }
                                }
                                if flags.get_seen() {
                                    let resp =
                                        ui.add(SizedButton::new(button_size, "Set not-seen"));
                                    if resp.clicked() {
                                        flags.set_seen(false);
                                        state.flags.insert(comment.id.clone(), flags.clone());
                                    }
                                } else {
                                    let resp = ui.add(SizedButton::new(button_size, "Set seen"));
                                    if resp.clicked() {
                                        flags.set_seen(true);
                                        state.flags.insert(comment.id.clone(), flags.clone());
                                    }
                                }
                                if flags.get_in_progress() {
                                    let resp =
                                        ui.add(SizedButton::new(button_size, "Not In Progress"));
                                    if resp.clicked() {
                                        flags.set_in_progress(false);
                                        state.flags.insert(comment.id.clone(), flags.clone());
                                    }
                                } else {
                                    let resp = ui.add(SizedButton::new(button_size, "In Progress"));
                                    if resp.clicked() {
                                        flags.set_in_progress(true);
                                        state.flags.insert(comment.id.clone(), flags.clone());
                                    }
                                }

                                if state.notify_data.notified(comment.id) {
                                    let reset_notify_button =
                                        SizedButton::new(button_size, "Remove Notify");
                                    let resp = ui.add(reset_notify_button);
                                    if resp.clicked() {
                                        state.notify_data.remove(comment.id);
                                    }
                                }
                            });

                            ui.end_row();
                        }
                    });
            });
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let state: AppState = self.state.read().clone();
        eframe::set_value(storage, eframe::APP_KEY, &state);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        autofetcher::update_comments(&mut self.comments_rx, &mut self.state.write().comments);

        egui::CentralPanel::default().show(ctx, |ui| {
            let total_width = ui.available_width();
            ui.vertical(|ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("HN \"Who is Hiring\" Evaluator");
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let mut state = self.state.write();
                    ui.with_layout(Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_max_width(400.0);
                        ui.label("Who is hiring URL");
                        ui.add(
                            egui::TextEdit::singleline(&mut state.hn_url)
                                .hint_text("https://news.ycombinator.com/item?id=..."),
                        );
                        ui.add_space(5.0);
                        ui.label("Google Gemini API Key");
                        ui.add(
                            egui::TextEdit::singleline(&mut state.api_key)
                                .hint_text("AIza...")
                                .password(true),
                        );
                        ui.add_space(5.0);
                        ui.separator();
                        if let Some(path) = &state.pdf_path
                            && let Some(basename) = PathBuf::from(path).file_name()
                        {
                            ui.label(format!("Selected PDF: {}", basename.to_string_lossy()));
                        } else {
                            ui.label(format!("PDF Missing"));
                        }

                        if state.eval_cache.is_usable() {
                            ui.label("Cache key: OK");
                        } else if state.eval_cache.is_some() {
                            ui.label("Cache key: Expired");
                        } else {
                            match &state.cache_key_error {
                                Some(err) => ui.label(
                                    egui::RichText::new(format!("Cache key error: {}", err))
                                        .color(egui::Color32::RED),
                                ),

                                None => ui.label("Cache key: None"),
                            };
                        }

                        ui.horizontal(|ui| {
                            ui.label("Auto fetch comments");
                            if toggle_ui(ui, &mut state.auto_fetch).changed() {
                                if state.auto_fetch {
                                    let url = state.hn_url.clone();
                                    state.auto_fetcher.enable(url, self.comments_tx.clone());
                                } else {
                                    state.auto_fetcher.disable();
                                }
                            }
                        });

                        if state.processing.enabled {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;

                                ui.label("Processing: ");
                                ui.colored_label(Color32::GREEN, state.processing.done.to_string());
                                ui.label("/");
                                ui.colored_label(Color32::RED, state.processing.error.to_string());
                                ui.label(format!("/{}", state.processing.total));
                            });
                        } else if state.processing.total > 0 {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;

                                ui.label("Processing (stopped): ");
                                ui.colored_label(Color32::GREEN, state.processing.done.to_string());
                                ui.label("/");
                                ui.colored_label(Color32::RED, state.processing.error.to_string());
                                ui.label(format!("/{}", state.processing.total));
                            });
                        } else {
                            ui.label("Processing stopped");
                        }
                        ui.label(format!("NTFY topic: {}", state.notify_data.topic));
                        let mut evaluated = 0;
                        for c in state.comments.iter() {
                            if state.evaluations.contains_key(&c.id) {
                                evaluated += 1;
                            }
                        }
                        ui.label(format!("Evaluated: {}/{}", evaluated, state.comments.len()));
                        ui.add_space(10.0);
                        ui.separator();
                        ui.with_layout(
                            Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
                            |ui| {
                                if ui.button("Select PDF").clicked() {
                                    if let Some(path) = rfd::FileDialog::new()
                                        .add_filter("PDF", &["pdf"])
                                        .pick_file()
                                    {
                                        state.pdf_path = Some(path.display().to_string());
                                    }
                                }
                                if ui.button("Get Ev Cache").clicked() {
                                    get_evaluation_cache(self.state.clone());
                                }

                                if ui.button("Process Comments").clicked() {
                                    process_comments(self.state.clone(), false);
                                }
                                if ui.button("Force Process Comments").clicked() {
                                    process_comments(self.state.clone(), true);
                                }
                                if ui.button("Batch Process").clicked() {
                                    state.processing.enabled = !state.processing.enabled;
                                    if state.processing.enabled {
                                        let _ = batch_process(&state.comments, self.state.clone());
                                    }
                                }
                                if ui.button("Nuke Evaluations").clicked() {
                                    state.evaluations = HashMap::new();
                                }
                                if ui.button("Export State").clicked() {
                                    if let Ok(json_str) = serde_json::to_string(&state.clone()) {
                                        let _ = std::fs::write("state.json", json_str);
                                    }
                                }
                            },
                        );

                        ui.add_space(10.0);
                    });
                    ui.with_layout(Layout::top_down_justified(egui::Align::Min), |ui| {
                        let tokens_count = estimate_accurate_tokens(&state.requirements);
                        ui.label(format!("Requirements (token count: {}):", tokens_count));
                        egui::Frame::group(ui.style())
                            .fill(ui.visuals().extreme_bg_color)
                            .inner_margin(2.0)
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(400.0) // Constraints the expansion
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::TextEdit::multiline(&mut state.requirements)
                                                .desired_rows(5)
                                                .frame(false)
                                                .hint_text("Requirements"),
                                        );
                                    });
                            });

                        ui.add_space(2.0);
                        ui.with_layout(Layout::right_to_left(egui::Align::Max), |ui| {
                            ui.scope(|ui| {
                                ui.set_max_width(150.0);
                                ui.text_edit_singleline(&mut state.search_string);
                            });
                            ui.label("Search: ");
                            ui.add_space(10.0);
                            let slider = egui::Slider::new(&mut state.min_score, 0..=100);
                            ui.add(slider);
                            ui.label("Min score: ");
                            ui.add_space(10.0);
                            ui.horizontal(|mut ui| {
                                toggle_ui(&mut ui, &mut state.hide_seen);
                                ui.label("Hide seen: ");
                            });
                            ui.add_space(5.0);
                            ui.horizontal(|mut ui| {
                                toggle_ui(&mut ui, &mut state.hide_in_progress);
                                ui.label("Hide in progress: ");
                            });
                        });
                        ui.add_space(5.0);
                    });
                });

                self.render_table(ui, total_width);
            });
        });
    }
}

pub fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([600.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "HN Evaluator",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

fn apply_everforest_theme(ctx: &egui::Context, dark_mode: bool) {
    use egui::{Color32, Shadow, Stroke, Visuals, style::WidgetVisuals};

    let mut visuals = if dark_mode {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    // --- Everforest Color Palette Definitions ---
    let bg_main = if dark_mode {
        Color32::from_rgb(45, 53, 59)
    } else {
        Color32::from_rgb(251, 248, 231)
    };
    let bg_dim = if dark_mode {
        Color32::from_rgb(51, 59, 66)
    } else {
        Color32::from_rgb(242, 239, 212)
    };
    let bg_ext = if dark_mode {
        Color32::from_rgb(40, 48, 54)
    } else {
        Color32::from_rgb(235, 230, 195)
    };
    let fg_text = if dark_mode {
        Color32::from_rgb(211, 198, 170)
    } else {
        Color32::from_rgb(92, 106, 115)
    };
    let accent = if dark_mode {
        Color32::from_rgb(167, 192, 128)
    } else {
        Color32::from_rgb(141, 161, 98)
    };
    let separator = if dark_mode {
        Color32::from_rgb(73, 81, 87)
    } else {
        Color32::from_rgb(223, 219, 185)
    };

    // --- 1. Global Surface & Text Overrides ---
    visuals.panel_fill = bg_main;
    visuals.window_fill = bg_dim;
    visuals.extreme_bg_color = bg_ext; // Text edits, progress bars, etc.
    visuals.faint_bg_color = bg_ext; // Zebra-striping in tables
    visuals.error_fg_color = Color32::from_rgb(230, 126, 128);
    visuals.warn_fg_color = Color32::from_rgb(214, 153, 105);

    // --- 2. High-Impact Widget States ---
    let setup_widget = |vis: &mut WidgetVisuals, fill: Color32, stroke: Color32, text: Color32| {
        vis.bg_fill = fill;
        vis.weak_bg_fill = fill;
        vis.bg_stroke = Stroke::new(1.0, stroke);
        vis.fg_stroke = Stroke::new(1.0, text);
        vis.corner_radius = egui::CornerRadius::same(3);
    };

    // Base state for most UI elements
    setup_widget(
        &mut visuals.widgets.noninteractive,
        bg_main,
        separator,
        fg_text,
    );

    // Buttons, Checkboxes, etc. (Static)
    setup_widget(&mut visuals.widgets.inactive, bg_dim, separator, fg_text);

    // Mouse over
    let hover_bg = if dark_mode {
        Color32::from_rgb(67, 76, 83)
    } else {
        Color32::from_rgb(230, 226, 195)
    };
    setup_widget(&mut visuals.widgets.hovered, hover_bg, accent, fg_text);

    // Clicked / Active
    let active_text = if dark_mode {
        Color32::from_rgb(45, 53, 59)
    } else {
        Color32::WHITE
    };
    setup_widget(&mut visuals.widgets.active, accent, accent, active_text);

    // --- 3. Selection & Shadows ---
    visuals.selection.bg_fill = accent.linear_multiply(0.2);
    visuals.selection.stroke = Stroke::new(1.0, accent);

    // Shadows help separate windows from background in pastel themes
    visuals.window_shadow = Shadow::NONE; // Pastel themes usually favor flat design
    visuals.popup_shadow = Shadow::NONE;

    ctx.set_visuals(visuals);
}
// Taken from egui demo
pub fn toggle_ui(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    // Widget code can be broken up in four steps:
    //  1. Decide a size for the widget
    //  2. Allocate space for it
    //  3. Handle interactions with the widget (if any)
    //  4. Paint the widget

    // 1. Deciding widget size:
    // You can query the `ui` how much space is available,
    // but in this example we have a fixed size widget based on the height of a standard button:
    let desired_size = ui.spacing().interact_size.y * egui::vec2(2.0, 1.0);

    // 2. Allocating space:
    // This is where we get a region of the screen assigned.
    // We also tell the Ui to sense clicks in the allocated region.
    let (rect, mut response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

    // 3. Interact: Time to check for clicks!
    if response.clicked() {
        *on = !*on;
        response.mark_changed(); // report back that the value changed
    }

    // Attach some meta-data to the response which can be used by screen readers:
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, "")
    });

    // 4. Paint!
    // Make sure we need to paint:
    if ui.is_rect_visible(rect) {
        // Let's ask for a simple animation from egui.
        // egui keeps track of changes in the boolean associated with the id and
        // returns an animated value in the 0-1 range for how much "on" we are.
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        // We will follow the current style by asking
        // "how should something that is being interacted with be painted?".
        // This will, for instance, give us different colors when the widget is hovered or clicked.
        let visuals = ui.style().interact_selectable(&response, *on);
        // All coordinates are in absolute screen coordinates so we use `rect` to place the elements.
        let rect = rect.expand(visuals.expansion);
        let radius = 0.5 * rect.height();
        ui.painter().rect(
            rect,
            radius,
            visuals.bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        // Paint the circle, animating it from left to right with `how_on`:
        let circle_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        let center = egui::pos2(circle_x, rect.center().y);
        ui.painter()
            .circle(center, 0.75 * radius, visuals.bg_fill, visuals.fg_stroke);
    }

    // All done! Return the interaction response so the user can check what happened
    // (hovered, clicked, ...) and maybe show a tooltip:
    response
}
