use remarque_native_replay::{compare_scenario, load_scenarios};
use std::path::Path;

fn main() -> Result<(), String> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let scenarios = load_scenarios(&fixtures)?;
    let failures = scenarios
        .iter()
        .filter_map(|scenario| compare_scenario(scenario).err())
        .collect::<Vec<_>>();
    if failures.is_empty() {
        println!("{} native scenarios match", scenarios.len());
        Ok(())
    } else {
        Err(format!(
            "{} native scenarios differ: {failures:#?}",
            failures.len()
        ))
    }
}
