use crate::{
    common_gui::*,
    demo,
    events::{self, Event, EventEnvelope},
    models::AppServiceDefault,
};
use eframe::egui::{self, Button, Color32, Layout, Widget};
use parking_lot::RwLock;
use std::{path::PathBuf, sync::Arc};

use crate::models::Usable;
use crate::tokens::estimate_accurate_tokens;

const TABLE_FONT_SIZE: f32 = 10.0;

pub struct App {
    pub event_handler: Arc<events::EventHandler>,
    pub state: Arc<RwLock<AppState>>,
}

macro_rules! event {
    ($self:expr, $e:expr) => {
        #[allow(unused)]
        use Event::*;
        let _ = $self.event_handler.sender().try_send(EventEnvelope {
            event: $e,
            span: tracing::Span::current(),
        });
    };
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
fn add_meter(ui: &mut egui::Ui, score: u32) {
    let (_, rect) = ui.allocate_space(ui.available_size());
    let painter = ui.painter();
    let num_bars = 30;
    let used_space = 0.4;
    let total_width = rect.width() * used_space;
    let x_offset = rect.left() + (rect.width() * ((1.0 - used_space) / 2.0));
    let bar_width = total_width;
    let bar_unit_height = rect.height() / num_bars as f32 - 2.0;
    let spacing = 2.0;
    let score_per_bars = 100 / num_bars;

    for i in 0..num_bars {
        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(x_offset, rect.bottom() - (i + 1) as f32 * (bar_unit_height + 1.0)),
            egui::vec2(bar_width, bar_unit_height - spacing),
        );

        let color = if (i as u32 * score_per_bars) < score {
            let t = (i as f32) / (num_bars as f32);
            // Non-linear transition: stay darker for longer before brightening
            let t_curve = t.powf(2.0);
            // Almost black/dark green to vibrant green
            egui::Color32::from_rgb(
                (20.0 * t_curve) as u8,
                (50.0 + 205.0 * t_curve) as u8,
                (20.0 * t_curve) as u8,
            )
        } else {
            //egui::Color32::from_gray(50)
            ui.style()
                .visuals
                .widgets
                .noninteractive
                .bg_fill
                .blend(Color32::BLACK.linear_multiply(0.2))
        };

        painter.rect_filled(bar_rect, 2.0, color);
    }
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
                            font_id: egui::FontId::proportional(TABLE_FONT_SIZE),
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
        if demo::is_demo() {
            return demo::app_new(&cc);
        }
        let mut state_rwlock: AppState = cc
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

        state_rwlock = AppState {
            auto_fetch: false,
            batch_processing: false,
            notifications_enabled: false,
            ..state_rwlock
        };
        let event_state = cc
            .storage
            .and_then(|storage| eframe::get_value::<events::State>(storage, "event_state"))
            .unwrap_or_default();
        //let event_state = events::State::hydrate_event_state(&state_rwlock);
        let state = Arc::new(RwLock::new(state_rwlock.clone()));

        let event_handler = events::EventHandler::new(event_state, Arc::new(AppServiceDefault {}));

        Self { event_handler, state }
    }

    fn render_table(&mut self, ui: &mut egui::Ui, _available_w: f32) {
        let event_state = self.event_handler.state.read().clone();
        let mut state = self.state.write();
        let indices: Vec<usize> = (0..(&event_state.comments).len()).collect();
        let mut indices: Vec<usize> = indices
            .into_iter()
            .filter(|i| {
                if state.search_string.is_empty() {
                    return true;
                }
                let mut matches = false;
                let comment = &event_state.comments[*i];
                //if let Some(evaluation) = state.evaluations.get(&comment.id) {}
                if let Some(t) = &comment.text
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
                let comment = &event_state.comments[*i];
                let Some(evaluation) = &event_state.evaluations.get(&comment.id) else {
                    return false;
                };
                evaluation.score >= state.min_score
            })
            .filter(|i| {
                if !state.hide_seen {
                    return true;
                }
                let comment = &event_state.comments[*i];
                let Some(flags) = &event_state.flags.get(&comment.id) else {
                    return true;
                };
                if flags.get_seen() { false } else { true }
            })
            .filter(|i| {
                if !state.hide_in_progress {
                    return true;
                }
                let comment = &event_state.comments[*i];
                let Some(flags) = &event_state.flags.get(&comment.id) else {
                    return true;
                };
                if flags.get_in_progress() { false } else { true }
            })
            .filter(|i| {
                let comment_id = &event_state.comments[*i].id;
                !&event_state.flags.get(&comment_id).map_or(false, |f| f.get_hide())
            })
            .collect();

        // Sorting logic
        indices.sort_by(|&a, &b| {
            let res = match state.sort_column {
                SortColumn::Score => {
                    let s_a = &event_state
                        .evaluations
                        .get(&event_state.comments[a].id)
                        .map(|e| e.score)
                        .unwrap_or(0);
                    let s_b = &event_state
                        .evaluations
                        .get(&event_state.comments[b].id)
                        .map(|e| e.score)
                        .unwrap_or(0);
                    s_a.cmp(&s_b)
                }
                SortColumn::Id => (&event_state.comments[a].id).cmp(&event_state.comments[b].id),
                SortColumn::CreatedAt => (&event_state.comments[a].created_at).cmp(&event_state.comments[b].created_at),
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
                    if ui.button("Created At").clicked() {
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
                    ui.label(format!("Showing {}/{}", indices.len(), event_state.comments.len()));
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
                    //.spacing((10.0, 50.0))
                    .striped(true)
                    .show(ui, |ui| {
                        ui.style_mut()
                            .text_styles
                            .insert(egui::TextStyle::Body, egui::FontId::proportional(10.0));
                        for row_idx in row_range {
                            let cols = cols.clone();
                            let idx = indices[row_idx];
                            let comment = &event_state.comments[idx].clone();
                            let eval = &event_state.evaluations.get(&comment.id);
                            let flags = &event_state.flags.get(&comment.id).copied().unwrap_or_default();

                            if flags.get_in_progress() {
                                ui.style_mut().visuals.override_text_color =
                                    Some(egui::Color32::from_rgb(100, 255, 100));
                                //ui.style_mut().visuals.override_text_color = Some(egui::Color32::GREEN);
                            } else if flags.get_seen() {
                                ui.style_mut().visuals.override_text_color = Some(egui::Color32::from_white_alpha(64));
                            } else {
                                ui.style_mut().visuals.override_text_color = None;
                            }
                            // CreatedAt
                            ui.table_cell(&cols, available_w, 0, |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.label(format!("{}", comment.created_at));
                                });
                                if let Some(ev) = eval
                                    && let Some(jd) = ev.job_description.clone()
                                {
                                    ui.vertical(|ui| {
                                        ui.add_space(10.0);

                                        if !jd.company_name.is_empty() && jd.company_name != "null" {
                                            ui.label(
                                                egui::RichText::new(format!("{}", jd.company_name))
                                                    .size(TABLE_FONT_SIZE),
                                            );
                                        }
                                        if !jd.job_title.is_empty() {
                                            ui.label(
                                                egui::RichText::new(format!("{}", jd.job_title)).size(TABLE_FONT_SIZE),
                                            );
                                        }
                                        ui.add_space(5.0);
                                        if !jd.technologies.is_empty() {
                                            ui.horizontal_top(|ui| {
                                                ui.set_max_width(130.0);
                                                ui.label(egui::RichText::new("Techs: ").size(TABLE_FONT_SIZE));
                                                let mut job = egui::text::LayoutJob::default();
                                                job.wrap.max_width = 130.0;
                                                for (i, tech) in jd.technologies.iter().enumerate() {
                                                    let color = match tech.to_lowercase().as_str() {
                                                        "go" | "golang" => Color32::from_rgb(0, 173, 216),
                                                        "rust" => Color32::from_rgb(183, 65, 14),
                                                        "elixir" => Color32::from_rgb(142, 85, 184),
                                                        "ruby" | "rails" => Color32::from_rgb(204, 0, 0),
                                                        "python" => Color32::from_rgb(255, 212, 59),
                                                        "typescript" => Color32::from_rgb(49, 120, 198),
                                                        "react" => Color32::from_rgb(97, 218, 251),
                                                        "java" => Color32::from_rgb(238, 20, 26),
                                                        _ => ui.visuals().text_color(),
                                                    };
                                                    let text = if i == jd.technologies.len() - 1 {
                                                        tech.to_string()
                                                    } else {
                                                        format!("{}, ", tech)
                                                    };
                                                    job.append(
                                                        &text,
                                                        0.0,
                                                        egui::text::TextFormat {
                                                            font_id: egui::FontId::proportional(TABLE_FONT_SIZE),
                                                            color,
                                                            ..Default::default()
                                                        },
                                                    );
                                                }
                                                ui.add(egui::Label::new(job).wrap());
                                            });
                                        }
                                        if !jd.compensation_currency.is_empty() && jd.compensation_currency != "null" {
                                            fn format_val(val: u64) -> String {
                                                if val >= 1_000_000 {
                                                    format!("{:.1}M", val as f64 / 1_000_000.0)
                                                } else if val >= 1_000 {
                                                    format!("{}k", val / 1_000)
                                                } else {
                                                    val.to_string()
                                                }
                                            }

                                            let suff = match (jd.compensation_min, jd.compensation_max) {
                                                (0, 0) => "".into(),
                                                (0, max) => format!("< {}", format_val(max)),
                                                (min, 0) => format!("> {}", format_val(min)),
                                                (min, max) => format!("{} - {}", format_val(min), format_val(max)),
                                            };
                                            if !suff.is_empty() {
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "Comp: {} {}",
                                                        suff, jd.compensation_currency
                                                    ))
                                                    .size(TABLE_FONT_SIZE),
                                                );
                                            }
                                        }
                                    });
                                }
                            });
                            // Comment
                            ui.table_cell(&cols, available_w, 1, |ui| {
                                ui.vertical(|ui| {
                                    ui.hyperlink_to(
                                        egui::RichText::new(format!("#{}", comment.id)).size(TABLE_FONT_SIZE),
                                        format!("https://news.ycombinator.com/item?id={}", comment.id),
                                    );
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(ui.available_width(), ui.available_height()),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            egui::ScrollArea::vertical()
                                                .id_salt(format!("comment_{}", comment.id))
                                                .auto_shrink([false, true])
                                                .show(ui, |ui| {
                                                    let mut job = egui::text::LayoutJob::default();
                                                    let text = comment.text.as_deref().unwrap_or("");
                                                    let lines: Vec<&str> = text.splitn(2, '\n').collect();

                                                    // Add first line
                                                    if let Some(first_line) = lines.first() {
                                                        job.append(
                                                            &format!("{}\n", first_line),
                                                            0.0,
                                                            egui::text::TextFormat {
                                                                font_id: egui::FontId::proportional(TABLE_FONT_SIZE),
                                                                color: ui.visuals().text_color(),
                                                                line_height: Some(20.0),
                                                                ..Default::default()
                                                            },
                                                        );
                                                    }

                                                    // Add Red Flags
                                                    if let Some(jd) =
                                                        eval.as_ref().and_then(|e| e.job_description.as_ref())
                                                        && !jd.red_flags.is_empty()
                                                    {
                                                        for flag in &jd.red_flags {
                                                            job.append(
                                                                &format!("• {}\n", flag),
                                                                0.0,
                                                                egui::text::TextFormat {
                                                                    font_id: egui::FontId::proportional(
                                                                        TABLE_FONT_SIZE,
                                                                    ),
                                                                    color: egui::Color32::from_rgb(201, 147, 58),
                                                                    line_height: Some(20.0),
                                                                    ..Default::default()
                                                                },
                                                            );
                                                        }
                                                        job.append("\n", 0.0, egui::text::TextFormat::default());
                                                    }

                                                    // Add remaining lines
                                                    if lines.len() > 1 {
                                                        job.append(
                                                            lines[1].trim_start(),
                                                            0.0,
                                                            egui::text::TextFormat {
                                                                font_id: egui::FontId::proportional(TABLE_FONT_SIZE),
                                                                color: ui.visuals().text_color(),
                                                                line_height: Some(20.0),
                                                                ..Default::default()
                                                            },
                                                        );
                                                    }

                                                    job.wrap.max_width = ui.available_width();
                                                    ui.add(egui::Label::new(job).wrap());
                                                });
                                        },
                                    );
                                });
                            });
                            // Evaluation
                            ui.table_cell(&cols, available_w, 2, |ui| {
                                if let Some(e) = eval {
                                    scrollable_row(ui, format!("eval_{}", comment.id), &e.evaluation);
                                } else {
                                    ui.centered_and_justified(|ui| ui.label("-"));
                                }
                            });

                            // Tech alignment
                            ui.table_cell(&cols, available_w, 3, |ui| {
                                if let Some(e) = eval {
                                    scrollable_row(ui, format!("tech_{}", comment.id), &e.technology_alignment);
                                } else {
                                    ui.centered_and_justified(|ui| ui.label("-"));
                                }
                            });

                            // Comp alignment
                            ui.table_cell(&cols, available_w, 4, |ui| {
                                if let Some(e) = eval {
                                    scrollable_row(ui, format!("comp_{}", comment.id), &e.compensation_alignment);
                                } else {
                                    ui.centered_and_justified(|ui| ui.label("-"));
                                }
                            });

                            // Score
                            ui.table_cell(&cols, available_w, 5, |ui| {
                                ui.vertical_centered(|ui| {
                                    if let Some(e) = eval {
                                        ui.add_space(10.0);
                                        ui.label(format!("{}", e.score));
                                        add_meter(ui, e.score);
                                    } else {
                                        ui.centered_and_justified(|ui| ui.label("-"));
                                    };
                                });
                            });

                            ui.table_cell(&cols, available_w, 6, |ui| {
                                let button_size = egui::vec2(available_w * cols[6] * 0.6, ui.spacing().interact_size.y);

                                let flags = &mut event_state.flags.get(&comment.id).copied().unwrap_or_default();

                                ui.add_space(10.0);
                                seen_button(&self.event_handler.sender(), button_size, comment, ui, flags);
                                inprogress_button(&self.event_handler.sender(), button_size, comment, ui, flags);
                                hide_button(&self.event_handler.sender(), button_size, comment, ui, flags);

                                ui.with_layout(Layout::bottom_up(egui::Align::Min).with_cross_justify(true), |ui| {
                                    //ui.set_width(button_size.x);
                                    // ui.painter().rect_filled(
                                    //     ui.max_rect(),
                                    //     0.0,
                                    //     Color32::from_rgb(255, 0, 0),
                                    // );
                                    ui.add_space(10.0);

                                    evaluate_button(&self.event_handler.sender(), button_size, &state, comment, ui);
                                    remove_notify_button(
                                        &self.event_handler.sender(),
                                        button_size,
                                        &event_state,
                                        comment,
                                        ui,
                                    );
                                });
                            });

                            ui.end_row();
                        }
                    });
            });
    }
}

fn remove_notify_button(
    tx: &tokio::sync::mpsc::Sender<EventEnvelope>,
    button_size: egui::Vec2,
    event_state: &events::State,
    comment: &crate::models::Comment,
    ui: &mut egui::Ui,
) {
    if (&event_state.notify_data).notified(comment.id) {
        let reset_notify_button = SizedButton::new(button_size, "Remove Notify");
        let resp = ui.add(reset_notify_button);
        if resp.clicked() {
            let _ = tx.try_send(EventEnvelope {
                event: Event::RemoveNotify(comment.id),
                span: tracing::Span::current(),
            });
        }
    }
}

fn inprogress_button(
    tx: &tokio::sync::mpsc::Sender<EventEnvelope>,
    button_size: egui::Vec2,
    comment: &crate::models::Comment,
    ui: &mut egui::Ui,
    flags: &mut Flags,
) {
    if flags.get_in_progress() {
        let resp = ui.add(SizedButton::new(button_size, "Not In Progress"));
        if resp.clicked() {
            flags.set_in_progress(false);
            let _ = tx.try_send(EventEnvelope {
                event: Event::FlagEventUpdate {
                    id: comment.id,
                    flag: flags.clone(),
                },
                span: tracing::Span::current(),
            });
        }
    } else {
        let resp = ui.add(SizedButton::new(button_size, "In Progress"));
        if resp.clicked() {
            flags.set_in_progress(true);
            let _ = tx.try_send(EventEnvelope {
                event: Event::FlagEventUpdate {
                    id: comment.id,
                    flag: flags.clone(),
                },
                span: tracing::Span::current(),
            });
        }
    }
}

fn seen_button(
    tx: &tokio::sync::mpsc::Sender<EventEnvelope>,
    button_size: egui::Vec2,
    comment: &crate::models::Comment,
    ui: &mut egui::Ui,
    flags: &mut Flags,
) {
    if flags.get_seen() {
        let resp = ui.add(SizedButton::new(button_size, "Set not-seen"));
        if resp.clicked() {
            flags.set_seen(false);

            let _ = tx.try_send(EventEnvelope {
                event: Event::FlagEventUpdate {
                    id: comment.id,
                    flag: flags.clone(),
                },
                span: tracing::Span::current(),
            });
        }
    } else {
        let resp = ui.add(SizedButton::new(button_size, "Set seen"));
        if resp.clicked() {
            flags.set_seen(true);
            let _ = tx.try_send(EventEnvelope {
                event: Event::FlagEventUpdate {
                    id: comment.id,
                    flag: flags.clone(),
                },
                span: tracing::Span::current(),
            });
        }
    }
}

fn hide_button(
    tx: &tokio::sync::mpsc::Sender<EventEnvelope>,
    button_size: egui::Vec2,
    comment: &crate::models::Comment,
    ui: &mut egui::Ui,
    flags: &mut Flags,
) {
    if flags.get_hide() {
        let resp = ui.add(SizedButton::new(button_size, "Unhide"));
        if resp.clicked() {
            flags.set_hide(false);
            let _ = tx.try_send(EventEnvelope {
                event: Event::FlagEventUpdate {
                    id: comment.id,
                    flag: flags.clone(),
                },
                span: tracing::Span::current(),
            });
        }
    } else {
        let resp = ui.add(SizedButton::new(button_size, "Hide"));
        if resp.clicked() {
            flags.set_hide(true);
            let _ = tx.try_send(EventEnvelope {
                event: Event::FlagEventUpdate {
                    id: comment.id,
                    flag: flags.clone(),
                },
                span: tracing::Span::current(),
            });
        }
    }
}

fn evaluate_button(
    tx: &tokio::sync::mpsc::Sender<EventEnvelope>,
    button_size: egui::Vec2,
    state: &AppState,
    comment: &crate::models::Comment,
    ui: &mut egui::Ui,
) {
    let button = SizedButton::new(button_size, "Evaluate");
    let resp = ui.add_enabled(true, button);
    if resp.clicked() {
        let _span = tracing::debug_span!("CLICK Evaluate").entered();
        let state = state.clone();
        let comment = comment.clone();
        let requirements = state.requirements.clone();
        let pdf_path = state.pdf_path.clone().unwrap_or_default();

        for e in vec![
            Event::Evaluate {
                try_cache: true,
                comment: comment.clone(),
                requirements,
                pdf_path,
                permit: None,
            },
            Event::FetchJobDescription {
                try_cache: true,
                id: comment.id,
                model: String::from(crate::models::MODEL),
                input: comment.text.clone().unwrap_or_default(),
                permit: None,
            },
        ] {
            let _ = tx.try_send(EventEnvelope {
                event: e,
                span: tracing::Span::current(),
            });
        }
    };
}
impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if demo::is_demo() {
            return;
        }
        let state: AppState = self.state.read().clone();
        let event_state = self.event_handler.state.read().clone();
        eframe::set_value(storage, "event_state", &event_state);
        eframe::set_value(storage, eframe::APP_KEY, &state);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let eh = self.event_handler.clone();
        let estate = eh.state.read().clone();

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

                        if estate.eval_cache.is_usable() {
                            ui.label("Cache key: OK");
                        } else if estate.eval_cache.is_some() {
                            ui.label("Cache key: Expired");
                        } else {
                            match &estate.cache_key_error {
                                Some(err) => ui.label(
                                    egui::RichText::new(format!("Cache key error: {}", err)).color(egui::Color32::RED),
                                ),

                                None => ui.label("Cache key: None"),
                            };
                        }
                        ui.horizontal(|ui| {
                            if toggle_ui(ui, &mut state.front_page_processing).changed() {
                                if state.front_page_processing {
                                    event!(self, FrontPageProcessingStart);
                                } else {
                                    event!(self, FrontPageProcessingEnd);
                                }
                            }
                            ui.label("Front Page Wait");
                        });

                        ui.horizontal(|ui| {
                            if toggle_ui(ui, &mut state.auto_fetch).changed() {
                                if state.auto_fetch {
                                    let url = state.hn_url.clone();
                                    event!(self, AutoFetchStart(url));
                                } else {
                                    event!(self, AutoFetchStop);
                                }
                            }
                            ui.label("Auto fetch comments");
                        });
                        ui.horizontal(|ui| {
                            if toggle_ui(ui, &mut state.batch_processing).changed() {
                                if state.batch_processing {
                                    event!(
                                        self,
                                        BatchProcessingStart {
                                            requirements: state.requirements.clone(),
                                            pdf_path: state.pdf_path.clone().unwrap_or_default(),
                                        }
                                    );
                                } else {
                                    event!(self, BatchProcessingStop);
                                }
                            }
                            ui.label("Batch process");
                        });

                        ui.horizontal(|ui| {
                            if toggle_ui(ui, &mut state.notifications_enabled).changed() {
                                if state.notifications_enabled {
                                    event!(self, SetNotifications { enabled: true });
                                } else {
                                    event!(self, SetNotifications { enabled: false });
                                }
                            }
                            ui.label("Notifications");
                        });

                        if estate.processing.enabled {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;

                                ui.label("Processing: ");
                                ui.colored_label(Color32::GREEN, estate.processing.done.to_string());
                                ui.label("/");
                                ui.colored_label(Color32::RED, estate.processing.error.to_string());
                                ui.label(format!("/{}", estate.processing.total));
                            });
                        } else if estate.processing.total > 0 {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;

                                ui.label("Processing (stopped): ");
                                ui.colored_label(Color32::GREEN, estate.processing.done.to_string());
                                ui.label("/");
                                ui.colored_label(Color32::RED, estate.processing.error.to_string());
                                ui.label(format!("/{}", estate.processing.total));
                            });
                        } else {
                            ui.label("Processing stopped");
                        }
                        ui.label(format!("NTFY topic: {}", estate.notify_data.topic));
                        let mut evaluated = 0;
                        for c in estate.comments.iter() {
                            if estate.evaluations.contains_key(&c.id) {
                                evaluated += 1;
                            }
                        }
                        ui.label(format!("Evaluated: {}/{}", evaluated, estate.comments.len()));
                        ui.add_space(10.0);
                        ui.separator();
                        ui.with_layout(Layout::left_to_right(egui::Align::Min).with_main_wrap(true), |ui| {
                            if ui.button("Select PDF").clicked() {
                                if let Some(path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                                    state.pdf_path = Some(path.display().to_string());
                                }
                            }

                            if ui.button("Process Comments").clicked() {
                                event!(
                                    self,
                                    CommentsProcess {
                                        url: state.hn_url.clone()
                                    }
                                );
                            }
                            if ui.button("Nuke Evaluations").clicked() {
                                event!(self, RemoveEvaluationAll);
                            }
                            if ui.button("Export State").clicked() {
                                if let Ok(json_str) = serde_json::to_string(&state.clone()) {
                                    let _ = std::fs::write("state.json", json_str);
                                }
                            }
                        });

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
        self.read_data_from_gui()
    }
}

impl App {
    fn read_data_from_gui(&self) {
        if let Some(state) = self.state.try_read()
            && let Some(event_state) = self.event_handler.state.try_read()
            && state.api_key != event_state.api_key.to_string()
        {
            event!(self, SyncApiKey(state.api_key.clone()));
        }
    }
}
pub fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([600.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native("HN Evaluator", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

fn apply_everforest_theme(ctx: &egui::Context, dark_mode: bool) {
    use egui::{Color32, Shadow, Stroke, Visuals, style::WidgetVisuals};

    let mut visuals = if dark_mode { Visuals::dark() } else { Visuals::light() };

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
    setup_widget(&mut visuals.widgets.noninteractive, bg_main, separator, fg_text);

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
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, ""));

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
