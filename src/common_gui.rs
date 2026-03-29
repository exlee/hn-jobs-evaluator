use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::autofetcher::AutoFetcher;
use crate::job_description::JobDescriptions;
use crate::notify::NotifyData;
use crate::{comments::Comment, evaluation::Evaluation};
use crate::{evaluation, events};

#[derive(Serialize, Deserialize, Debug)]
pub struct RunSpec {
    pub hn_url: String,
    pub pdf_path: String,
    pub api_key: String,
    pub requirements: String,
}

#[derive(Default, Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Copy)]
pub struct Flags(u8);
use paste::paste;
macro_rules! bitset {
    ($const:ident = $name:ident) => {
        paste! {
            pub fn [<set_ $name>](&mut self, value: bool) {
                if value {
                    self.0 |= Self::$const;
                } else {
                    self.0 &= !Self::$const;
                }
            }
            pub fn [<get_ $name>](&self) -> bool {
                (self.0 & Self::$const) != 0
            }
        }
    };
}
impl Flags {
    const HIDE: u8 = 1 << 0;
    const SEEN: u8 = 1 << 1;
    const IN_PROGRESS: u8 = 1 << 2;
    bitset!(HIDE = hide);
    bitset!(SEEN = seen);
    bitset!(IN_PROGRESS = in_progress);
}

impl PartialOrd for Flags {
    fn partial_cmp(&self, _other: &Self) -> Option<std::cmp::Ordering> {
        if self.0 == Self::HIDE || self.0 == Self::SEEN {
            Some(std::cmp::Ordering::Greater)
        } else {
            None
        }
    }
}
impl Ord for Flags {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.partial_cmp(other) {
            None => std::cmp::Ordering::Equal,
            Some(v) => v,
        }
    }
}
#[derive(Serialize, Deserialize, Default, Clone)]
pub enum SortColumn {
    #[default]
    CreatedAt,
    Score,
    Id,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct ProcessingData {
    pub enabled: bool,
    pub total: usize,
    pub error: usize,
    pub done: usize,
}
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct AppState {
    pub hn_url: String,
    pub search_string: String,
    pub requirements: String,
    pub pdf_path: Option<String>,
    pub api_key: String,
    pub sort_column: SortColumn,
    pub descending: bool,
    pub min_score: u32,
    pub hide_seen: bool,
    pub hide_in_progress: bool,
    pub auto_fetch: bool,
    pub batch_processing: bool,
    pub notifications_enabled: bool,
}

// pub struct AppState {
//     pub hn_url: String,
//     pub search_string: String,
//     pub requirements: String,
//     pub pdf_path: Option<String>,
//     pub api_key: String,
//     pub sort_column: SortColumn,
//     pub descending: bool,
//     pub min_score: u32,
//     pub hide_seen: bool,
//     pub hide_in_progress: bool,
// }
