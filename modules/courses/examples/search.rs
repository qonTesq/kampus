//! Manual smoke: cargo run -p kampus-courses --example search -- school.yaml SUBJECT
use kampus_core::{
    config::SchoolConfig,
    contracts::{CallContext, ModuleInit, Principal},
    runtime::SharedRuntime,
};
use std::{path::Path, sync::Arc};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).ok_or("school.yaml path required")?;
    let subject = args.get(2).ok_or("subject required")?;
    let school = Arc::new(SchoolConfig::load(Path::new(path))?);
    let module = kampus_courses::module(&ModuleInit {
        school: Arc::clone(&school),
        runtime: Arc::new(SharedRuntime::new()),
    })?;
    let ctx = CallContext {
        school_id: school.school.id.clone(),
        principal: Principal::guest(),
        request_id: "manual-smoke".into(),
        now: chrono::Utc::now(),
        policy_version: 0,
        binding_generation: None,
    };
    let result = module
        .call(
            "search_courses",
            serde_json::json!({"subject":subject}),
            &ctx,
        )
        .await?;
    // Deliberately report counts only, never query or course content.
    assert_eq!(result.data["page"], 1);
    assert!(result.data["sections"].is_array());
    Ok(())
}
