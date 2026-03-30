// pub mod app_service;
// pub mod autofetcher;
// pub mod batch_processor;
// pub mod comments;
// pub mod common_gui;
// pub mod evaluation;
// pub mod events;
// pub mod gui;
// pub mod job_description;
// pub mod notify;
pub mod common_gui;
pub mod demo;
pub mod events;
pub mod gui;
pub mod tokens;

// The backend module is private to the crate, or pub(crate)
// It contains the implementation details that GUI should not touch.
mod backend {
    pub mod app_service;
    pub mod autofetcher;
    pub mod batch_processor;
    pub mod comments;
    pub mod evaluation;
    pub mod job_description;
    pub mod notify;
}

// Publicly re-export only the structs/types for the rest of the crate (and GUI)
pub mod models {
    pub use crate::backend::app_service::{AppService, AppServiceDefault, async_res};
    pub use crate::backend::batch_processor::BatchProcessor;
    pub use crate::backend::comments::Comment;
    pub use crate::backend::evaluation::{Evaluation, EvaluationCache, MODEL, Usable};
    pub use crate::backend::job_description::{JobDescription, JobDescriptions};
}

#[cfg(test)]
mod tests;
