use std::collections::HashMap;

use opentelemetry_otlp::WithHttpConfig as _;
use tokio;

use jobs::*;
// mod app_service;
// mod appstate_evaluation;
// mod autofetcher;
// mod batch_processor;
// mod comments;
// mod common_gui;
// mod evaluation;
// mod events;
// mod gui;
// mod job_description;
// mod notify;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    deadlock_detection();
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    let (trace_provider, log_provider) = init_tracer()?;
    tracing::info!("Starting application.");
    let _ = gui::main();
    trace_provider.shutdown()?;
    log_provider.shutdown()?;
    Ok(())
}
fn deadlock_detection() {
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let deadlocks = parking_lot::deadlock::check_deadlock();
            if !deadlocks.is_empty() {
                for (i, threads) in deadlocks.iter().enumerate() {
                    eprintln!("Deadlock #{i}");
                    for t in threads {
                        eprintln!("Thread id: {:?}\n{:#?}", t.thread_id(), t.backtrace());
                    }
                }
            }
        }
    });
}

use opentelemetry::KeyValue;
use opentelemetry::global;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    logs::SdkLoggerProvider,
    resource::Resource,
    trace::{RandomIdGenerator, Sampler, SdkTracerProvider},
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn make_resource() -> Resource {
    Resource::builder()
        .with_attribute(KeyValue::new("service.name", "jobs"))
        .build()
}

fn make_headers(api_key: &str) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    headers.insert("x-honeycomb-team".to_string(), api_key.to_string());
    headers.insert("x-honeycomb-dataset".to_string(), "jobs".to_string());
    headers
}
pub fn init_tracer() -> anyhow::Result<(SdkTracerProvider, SdkLoggerProvider)> {
    let resource = make_resource();

    let (span_exporter, log_exporter) = match std::env::var("HONEYCOMB_API_KEY").ok() {
        Some(api_key) => {
            let headers = make_headers(&api_key);
            let spans = SpanExporter::builder()
                .with_http()
                .with_endpoint("https://api.eu1.honeycomb.io/v1/traces")
                .with_headers(headers.clone())
                .build()?;
            let logs = LogExporter::builder()
                .with_http()
                .with_endpoint("https://api.eu1.honeycomb.io/v1/logs")
                .with_headers(headers)
                .build()?;
            (spans, logs)
        }
        None => {
            let spans = SpanExporter::builder()
                .with_tonic()
                .with_endpoint("http://localhost:4317")
                .build()?;
            let logs = LogExporter::builder()
                .with_tonic()
                .with_endpoint("http://localhost:4317")
                .build()?;
            (spans, logs)
        }
    };

    let log_provider = SdkLoggerProvider::builder()
        .with_batch_exporter(log_exporter)
        .with_resource(resource.clone())
        .build();

    let trace_provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_sampler(Sampler::AlwaysOn)
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(resource)
        .build();

    global::set_tracer_provider(trace_provider.clone());

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_opentelemetry::layer().with_tracer(global::tracer("job")))
        .with(OpenTelemetryTracingBridge::new(&log_provider))
        .with(tracing_tree::HierarchicalLayer::new(2))
        .init();

    Ok((trace_provider, log_provider))
}
