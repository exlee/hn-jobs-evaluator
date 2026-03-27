use std::{collections::HashMap, time::Duration};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{Receiver, Sender};

use crate::autofetcher::AutoFetcher;
use crate::comments;
use crate::job_description::{JobDescription, JobDescriptions};
use crate::notify::NotifyData;
use crate::{comments::Comment, evaluation::Evaluation};

#[derive(Serialize, Deserialize, Debug)]
pub struct RunSpec {
    pub hn_url: String,
    pub pdf_path: String,
    pub api_key: String,
    pub requirements: String,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct EvaluationCache {
    pub key: String,
    pub timestamp: chrono::DateTime<Utc>,
    pub ttl: Duration,
}
#[derive(Default, Serialize, Deserialize, Clone, PartialEq, Eq)]
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
pub trait Usable {
    fn is_usable(&self) -> bool;
}
impl Usable for Option<EvaluationCache> {
    fn is_usable(&self) -> bool {
        if self.is_none() {
            return false;
        }

        self.as_ref().unwrap().is_usable()
    }
}
impl Usable for EvaluationCache {
    fn is_usable(&self) -> bool {
        let td = Utc::now() - self.timestamp;
        if td.num_seconds() > self.ttl.as_secs() as i64 {
            false
        } else {
            true
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
    pub notify_data: NotifyData,
    pub processing: ProcessingData,
    pub eval_cache: Option<EvaluationCache>,
    pub cache_key_error: Option<String>,
    pub flags: HashMap<u32, Flags>,
    pub hn_url: String,
    pub search_string: String,
    pub requirements: String,
    pub pdf_path: Option<String>,
    pub api_key: String,
    pub comments: Vec<Comment>,
    pub evaluations: std::collections::HashMap<u32, Evaluation>,
    pub sort_column: SortColumn,
    pub descending: bool,
    pub min_score: u32,
    pub hide_seen: bool,
    pub hide_in_progress: bool,
    pub auto_fetch: bool,
    pub auto_fetcher: AutoFetcher,
    pub job_descriptions: JobDescriptions,
}
