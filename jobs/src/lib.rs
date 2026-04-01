pub mod common_gui;
pub mod demo;
pub mod events;
pub mod gui;
pub mod tokens;

mod backend {
    pub mod app_service;
    pub mod autofetcher;
    pub mod batch_processor;
    pub mod comments;
    pub mod comments_algolia;
    pub mod comments_firebase;
    pub mod evaluation;
    pub mod front_page;
    pub mod job_description;
    pub mod notify;
}

// Publicly re-export only the structs/types for the rest of the crate (and GUI)
pub mod models {
    pub use crate::backend::app_service::{AppService, AppServiceDefault, async_res};
    pub use crate::backend::batch_processor::BatchProcessor;
    pub use crate::backend::comments::Comment;
    pub use crate::backend::evaluation::{Evaluation, EvaluationCache, MODEL, Usable};
    pub use crate::backend::front_page::Story;
    pub use crate::backend::job_description::{JobDescription, JobDescriptions};
}

pub mod api {

    pub mod firebase {
        pub use crate::backend::comments_firebase::get_comments;
    }
}
#[cfg(test)]
mod tests;
