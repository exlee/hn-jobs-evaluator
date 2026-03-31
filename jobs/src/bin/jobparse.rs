use jobs::models::{AppService, AppServiceDefault};

const MODEL: &str = "gemini-3.1-flash-lite-preview";
fn main() {
    let app_service = AppServiceDefault {};
    let api_key = std::env::var("GOOGLE_API_KEY").expect("GOOGLE_API_KEY must be set");
    let llm_config = llmuxer::LlmConfig {
        provider: llmuxer::Provider::Gemini,
        api_key,
        base_url: None,
        model: MODEL.to_string(),
    };

    let mut clipboard = arboard::Clipboard::new().expect("Failed to initialize clipboard");
    let input = clipboard.get_text().expect("Clipboard is empty");
    if input.split_whitespace().count() < 3 {
        panic!("Clipboard content must have at least 3 words");
    }

    match app_service.parse_job_description(llm_config, &input) {
        Ok(job) => {
            println!("Parsed Job Description:");
            println!("Company: {}", job.company_name);
            println!("Title: {}", job.job_title);
            println!("Location: {}", job.location);
            println!(
                "Compensation: {} - {} {}",
                job.compensation_min, job.compensation_max, job.compensation_currency
            );
            println!("Technologies: {:?}", job.technologies);
            println!("Work Type: {}", job.work_type);
        }
        Err(e) => eprintln!("Error parsing job: {}", e),
    }
}
