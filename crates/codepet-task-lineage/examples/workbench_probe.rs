//! Read-only smoke probe; prints counts without conversation titles or paths.
use codepet_task_lineage::{sources::local, workbench};
fn main() -> Result<(), String> {
    let home = local::directory(local::CODEX_ID)?;
    let catalog = workbench::projects(&home)?;
    let projects = catalog["projects"].as_array().unwrap();
    let selected: Vec<String> = projects
        .iter()
        .filter(|p| {
            p["name"]
                .as_str()
                .is_some_and(|n| n == "codepet" || n == "Code Pet")
        })
        .take(1)
        .filter_map(|p| p["id"].as_str().map(str::to_owned))
        .collect();
    println!(
        "Local project catalog: {} projects, {} warnings",
        projects.len(),
        catalog["warnings"].as_array().unwrap().len()
    );
    if selected.is_empty() {
        println!("No codepet project in catalog; scanning skipped.");
        return Ok(());
    }
    let snapshot = workbench::scan(&home, &selected, |done, total| {
        println!("Git: {done}/{total}");
        true
    })?;
    let rows = snapshot["worktrees"].as_array().unwrap();
    println!(
        "Result: {} worktrees, {} conversations, {} failed trees, {} warnings",
        rows.len(),
        rows.iter()
            .map(|r| r["conversations"].as_array().unwrap().len())
            .sum::<usize>(),
        rows.iter()
            .filter(|r| r["git"]["error"].is_string())
            .count(),
        snapshot["warnings"].as_array().unwrap().len()
    );
    Ok(())
}
