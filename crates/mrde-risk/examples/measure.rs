//! Offline diagnostic: cargo run -p mrde-risk --example measure -- WORKTREE BASE_SHA HEAD_SHA.
//! Outputs a RiskReport as JSON. Uses defaults; never accesses credentials or fetches Git.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() == 3, "Usage: measure WORKTREE BASE_SHA HEAD_SHA");
    let context = mrde_risk::context(
        std::path::Path::new(&args[0]),
        &args[1],
        &args[2],
        Default::default(),
    )
    .await?;
    let report = mrde_risk::analyze(context).await;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
