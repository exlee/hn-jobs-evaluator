use llmuxer::ResponseShape;
use serde::{Deserialize, Serialize};

const MODEL: &str = "gemini-3.1-flash-lite-preview";

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JobDescription {
    pub company_name: String,
    pub job_title: String,
    pub description: String,
    pub size: String,
    pub technologies: Vec<String>,
    pub location: String,
    pub compensation_min: u64,
    pub compensation_max: u64,
    pub compensation_currency: String,
    pub work_type: String,
    pub url: String,
    pub posted_at: String,
    pub experience_level: String,
    pub apply_at_url: String,
    pub apply_at_email: String,
    pub red_flags: Vec<String>,
}

pub fn parse_job_description(
    llm_config: llmuxer::LlmConfig,
    input: &str,
) -> Result<JobDescription, String> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "company_name": { "type": "string" },
            "job_title": { "type": "string" },
            "size": { "type": "string" },
            "technologies": { "type": "array", "items": { "type": "string" } },
            "location": { "type": "string" },
            "compensation_min": { "type": "integer" },
            "compensation_max": { "type": "integer" },
            "compensation_currency": { "type": "string" },
            "work_type": { "type": "string" },
            "url": { "type": "string" },
            "posted_at": { "type": "string" },
            "experience_level": { "type": "string" },
            "apply_at_url": { "type": "string" },
            "apply_at_email": { "type": "string" },
            "red_flags": { "type": "array", "items": { "type": "string" } },
        },
    });
    let json_schema = serde_json::to_value(schema).unwrap();
    let response_shape = ResponseShape::Json(json_schema);
    let llm_client = llmuxer::LlmClientBuilder::new()
        .config(llm_config)
        .response_shape(response_shape)
        .instruction(
            r"
            Read following job description and respond with JSON of defined shape.
        ",
        )
        .build()
        .expect("Couldn't build the client");
    let llm_result = llm_client.query(input).json::<JobDescription>();

    if let Err(llm_error) = llm_result {
        return Err(llm_error.to_string());
    }
    match llm_result {
        Err(e) => Err(e.to_string()),
        Ok(mut v) => {
            v.description = String::from(input);
            Ok(v)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use llmuxer::{LlmConfig, Provider};
    use std::env;

    #[cfg(feature = "integration-tests")]
    #[test]
    fn test_parse_job_description() {
        let api_key = env::var("TEST_GOOGLE_API_KEY")
            .expect("TEST_GOOGLE_API_KEY must be set to run this test");

        let input = r#"
            We are looking for a Senior Rust Developer at Oxide Computer Company.
            The team size is about 50 people.
            We use Rust, Nix, and Tailscale.
            Remote work is allowed, we're at NYC.
            Compensation is 150k USD per year.

            Work is only at night.

            Apply to test@testjobs.test
        "#;

        let result = parse_job_description(
            LlmConfig {
                provider: Provider::Gemini,
                api_key,
                base_url: None,
                model: String::from(MODEL),
            },
            input,
        );

        assert!(result.is_ok(), "Parsing failed: {:?}", result.err());

        let job = result.unwrap();
        assert_eq!(job.company_name, "Oxide Computer Company");
        assert!(job.job_title.contains("Rust Developer"));
        assert!(job.technologies.contains(&"Rust".to_string()));
        assert_eq!(job.work_type, "Remote");
    }
}
