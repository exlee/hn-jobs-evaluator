use std::{
    env,
    sync::{Arc, OnceLock},
};

use chrono::Utc;
use parking_lot::RwLock;

use crate::{
    backend::autofetcher::AutoFetcher,
    common_gui, events, gui,
    models::{AppService, Comment, Evaluation, JobDescription, async_res},
};

static DEMO: OnceLock<bool> = OnceLock::new();
pub fn is_demo() -> bool {
    *DEMO.get_or_init(|| env::var("DEMO").is_ok())
}

pub struct AppServiceDemo {}
impl AppService for AppServiceDemo {
    fn get_comments_from_url(&self, _url: &str, _force: bool) -> async_res!(Vec<Comment>) {
        let mut counter = 21321;
        let off: fn(i64) -> chrono::DateTime<Utc> = |min| Utc::now() - chrono::Duration::minutes(min);
        let inc = move || {
            counter += 1;
            counter
        };
        std::thread::sleep(std::time::Duration::from_millis(1500));
        Box::pin(generate_comments(off, inc))
    }

    fn evaluate_comment_cached(
        &self,
        _comment: &crate::models::Comment,
        _ev_cache: &crate::models::EvaluationCache,
        _api_key: &str,
    ) -> async_res!(anyhow::Result<Evaluation>) {
        Box::pin(generate_evaluation())
    }

    fn create_evaluation_cache(
        &self,
        _api_key: &str,
        _pdf_path: &std::path::Path,
        _requirements: &str,
        _ttl: std::time::Duration,
    ) -> async_res!(Result<String, String>) {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        Box::pin(async { Ok(String::from("EVALUATION_CACHE_STRING")) })
    }

    fn parse_job_description(
        &self,
        _llm_config: llmuxer::LlmConfig,
        _input: &str,
    ) -> Result<crate::models::JobDescription, String> {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        generate_job_description()
    }

    fn notify_evaluation(
        &self,
        _id: u32,
        _notify_data: &mut crate::backend::notify::NotifyData,
        evaluation: &Evaluation,
    ) -> anyhow::Result<()> {
        let technologies = evaluation
            .job_description
            .clone()
            .map(|jd| jd.technologies)
            .map(|t| t.join(", "))
            .unwrap_or(String::from("UNKNOWN"));
        let message = format!(
            "Evaluation Score: {}\n\nTechnologies: {}, Eval: {}\nTech: {}\nComp: {}",
            evaluation.score,
            technologies,
            evaluation.evaluation,
            evaluation.technology_alignment,
            evaluation.compensation_alignment
        );

        let company_name = evaluation
            .job_description
            .clone()
            .map(|jd| jd.company_name)
            .unwrap_or(String::from("Unknown company"));

        let title = format!("New Job {}", company_name);
        println!("Notification: {}\n{}", title, message);
        Ok(())
    }
}

async fn generate_evaluation() -> anyhow::Result<Evaluation> {
    use rand::RngExt;
    use rand::seq::IndexedRandom as _;

    let mut rng = rand::rng();

    let evaluations = [
        "Strong alignment with the role requirements. The candidate's experience matches well with our tech stack.",
        "Good match overall, though there are some areas that would need additional training.",
        "Excellent fit for the position. The technical skills align perfectly with our needs.",
        "Moderate alignment. Some relevant experience but gaps in key areas.",
        "Very promising candidate with strong relevant background. Would be a great addition to the team.",
    ];

    let tech_alignments = [
        "Excellent match with required technologies. Proficient in all listed stacks.",
        "Good technology alignment. Has experience with most required tools.",
        "Partial technology match. Some skills align while others need development.",
        "Strong alignment with primary tech requirements. Secondary skills may need upskilling.",
        "Technology stack aligns well with position needs. Relevant certifications a plus.",
    ];

    let comp_alignments = [
        "Compensation expectations align well with our budget range.",
        "Slightly above our range but negotiable for the right candidate.",
        "Within acceptable range for the experience level provided.",
        "Above budget but could be considered for exceptional fit.",
        "Compensation expectations match our senior level offerings.",
    ];

    std::thread::sleep(std::time::Duration::from_millis(1500));
    Ok(Evaluation {
        evaluation: evaluations.choose(&mut rng).unwrap().to_string(),
        technology_alignment: tech_alignments.choose(&mut rng).unwrap().to_string(),
        compensation_alignment: comp_alignments.choose(&mut rng).unwrap().to_string(),
        score: rng.random_range(1..=100),
        job_description: Some(generate_job_description().map_err(|e| anyhow::anyhow!(e))?),
    })
}

#[rustfmt::skip]
pub fn generate_job_description() -> Result<JobDescription, String> {

    use rand::seq::IndexedRandom as _;
    use rand::RngExt;
    let mut rng = rand::rng();
    let company_names = [
        "TechCorp", "DataFlow", "CloudNine", "InnovateX", "SoftSystems", "WebScale", "Cyberdyne", "GlobalNet", "ByteSize", "Streamline",
    ];
    let job_titles = [
        "Software Engineer", "DevOps Engineer", "Data Scientist", "Frontend Developer", "Product Manager", "Backend Developer", "Site Reliability Engineer",
    ];
    let locations = [
        "New York", "London", "Berlin", "San Francisco", "Paris", "Austin", "Amsterdam", "Chicago", "Dublin", "Seattle",
    ];
    let technologies = ["Rust", "Python", "Go", "TypeScript", "AWS", "Kubernetes", "React"];
    let work_types = ["Remote", "Hybrid", "On-site"];
    let compensations = [
        100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200, 220, 250, 300, 350,
    ];
    let currencies = ["USD", "EUR", "GBP"];
    let levels = ["Junior", "Mid", "Senior", "Lead", "Principal"];
    let flags = ["High turnover rate", "Micro-management culture", "Legacy codebase", "Tight deadlines"];
    let descriptions = [
        "Join our innovative team building the future of scaleable infrastructure.",
        "We are looking for a passionate individual to help us ship high-quality code.",
        "Work on cutting-edge problems with a collaborative and diverse group of engineers.",
        "Exciting opportunity to shape our product roadmap and technical direction."
    ];

    let comp_min = *compensations.choose(&mut rng).unwrap();
    let comp_max = comp_min + 50;

    Ok(JobDescription {
        company_name: company_names.choose(&mut rng).unwrap().to_string(),
        job_title: job_titles.choose(&mut rng).unwrap().to_string(),
        description: descriptions.choose(&mut rng).unwrap().to_string(),
        size: "Mid-size".to_string(),
        technologies: vec![technologies.choose(&mut rng).unwrap().to_string()],
        location: locations.choose(&mut rng).unwrap().to_string(),
        compensation_min: comp_min * 1000,
        compensation_max: comp_max * 1000,
        compensation_currency: currencies.choose(&mut rng).unwrap().to_string(),
        work_type: work_types.choose(&mut rng).unwrap().to_string(),
        url: "https://example.com/jobs".to_string(),
        posted_at: (Utc::now() - chrono::Duration::minutes(rand::rng().random_range(0..4320)))
            .to_rfc3339(),
        experience_level: levels.choose(&mut rng).unwrap().to_string(),
        apply_at_url: "https://example.com/apply".to_string(),
        apply_at_email: "jobs@example.com".to_string(),
        red_flags: vec![flags.choose(&mut rng).unwrap().to_string()],
    })
}

#[rustfmt::skip]
async fn generate_comments(
    off: fn(i64) -> chrono::DateTime<Utc>,
    mut inc: impl FnMut() -> u32,
) -> Vec<Comment> {
    let companies = [
        "TechCorp", "DataFlow", "CloudNine", "InnovateX", "SoftSystems", "WebScale", "Cyberdyne",
        "GlobalNet", "ByteSize", "Streamline", "Nexus", "Vertex", "Prime", "Apex", "Zenith", "Quantum",
        "Orbit", "Flux", "Sync", "Pulse", "Aura", "Nova", "Terra", "Luna", "Sol", "Vortex", "Vector",
        "Helix", "Axis", "Core", "Grid", "Base", "Link", "Node", "Flow", "Shift", "Spark", "Bolt",
        "Dash", "Rush", "Scale", "Range", "Scope", "View", "Sight", "Mind", "Brain", "Logic", "Code",
        "Data", "Soft", "Hard", "Fast", "Slow", "High", "Deep", "Wide", "Open", "Blue", "Gold",
        "Silver", "Bronze", "Iron", "Steel", "Copper", "Zinc", "Neon", "Argon", "Helium", "Ozone",
        "Atom", "Ion", "Ray", "Beam", "Wave", "Field", "Force", "Power", "Volt", "Watt", "Amp", "Ohm",
        "Farad", "Henry", "Tesla", "Weber", "Joule", "Pascal", "Bar", "Torr", "Lux", "Cand", "Mole",
        "Kilo", "Mega", "Giga", "Tera", "Peta", "Exa", "Zetta",
    ];
    let positions = [
        "Software Engineer", "DevOps Engineer", "Data Scientist", "Frontend Developer",
        "Product Manager", "Backend Developer", "Site Reliability Engineer",
    ];
    let locations = [
        "New York", "London", "Berlin", "San Francisco", "Paris", "Austin", "Amsterdam", "Chicago",
        "Dublin", "Seattle",
    ];
    let tech = [
        "Rust", "Python", "Go", "TypeScript", "AWS", "Kubernetes", "React", "PostgreSQL", "Docker",
        "Java", "C++", "C#", "Node.js", "Vue", "Angular", "GraphQL", "MongoDB", "Redis", "Terraform",
        "Azure", "GCP", "Ruby", "PHP", "Swift", "Kotlin", "Scala", "Elixir", "Flutter", "Tailwind",
    ];
    let work_options = ["Remote", "Hybrid", "On-site"];
    let compensations = [
        "$100k", "$110k", "$120k", "$130k", "$140k", "$150k", "$160k", "$170k", "$180k", "$190k",
        "$200k", "$220k", "$250k", "$300k", "$350k",
    ];
    let mut comments = Vec::with_capacity(100);
    let flags = [
        "\nNote: High turnover mentioned by former employees.",
        "\nWarning: Rapid expansion has led to team instability.",
        "\nNote: Management style is reported as micro-managing.",
        "\nWarning: Mixed reviews regarding company culture and work-life balance.",
        "\nNote: Recent reports suggest significant internal restructuring.",
    ];

    for i in 0..100 {
        let company = companies[i % companies.len()];
        let pos = positions[i % positions.len()];
        let loc = locations[i % locations.len()];
        let work = work_options[i % work_options.len()];
        let tech_str = if i % 10 < 7 {
            format!(" | Stack: {}", tech[i % tech.len()])
        } else {
            "".into()
        };
        let loc_str = format!(" | {} | {}", loc, work);
        let pay = format!(" | {}", compensations[i % compensations.len()]);
        let flag = if i % 100 < 5 {
            flags[i % flags.len()]
        } else {
            ""
        };

        let text = format!(
            "Hiring at {} for {}.{}{}{}.{}",
            company, pos, loc_str, tech_str, pay, flag
        );

        comments.push(Comment {
            created_at: off(i as i64),
            id: inc(),
            author: format!("{}_jobs", company).into(),
            text: text.into(),
            parent: 1,
            children: vec![],
        });
    }
    comments
}
pub fn app_new(_cc: &eframe::CreationContext<'_>) -> gui::App {
    let state: events::State = events::State {
        auto_fetcher: AutoFetcher {
            handle: None,
            app_service: Arc::new(AppServiceDemo {}),
        },
        ..Default::default()
    };
    let event_handler = events::EventHandler::new(state, Arc::new(AppServiceDemo {}));
    let state = common_gui::AppState {
        requirements: "I am currently seeking a fully remote position based in Europe. My primary focus is to join an organization that values efficient software development practices and allows me to contribute from a location within the European time zone, ensuring a healthy work-life balance while maintaining professional collaboration with distributed teams.\n\nRegarding compensation, I am looking for a salary of approximately 100,000€ per year. I believe this reflects the market rate for high-level contributions and ensures that I can provide significant value to the company while maintaining my standard of living in my region.\n\nTechnologically, I am specifically looking for positions that utilize the Ruby on Rails stack. I have extensive experience with the framework and its associated ecosystem, including PostgreSQL databases and related backend technologies. I thrive in environments where clean, maintainable code is prioritized and where I can leverage my deep knowledge of the Rails lifecycle.\n\nOn a non-technical note, I have a strong preference against working in the education sector. I am looking for challenges in industry, SaaS, or enterprise software development, as these domains align better with my professional goals and previous experience in building high-performance systems.\n\nFinally, I am committed to continuous learning and working with high-caliber teams. I am looking for a company culture that fosters growth, technical excellence, and transparent communication, allowing me to fully integrate into the product roadmap and help drive technical initiatives forward.".into(),
        pdf_path: Some(String::new()),
        ..Default::default()
    };
    gui::App {
        event_handler: event_handler,
        state: Arc::new(RwLock::new(state)),
    }
}
