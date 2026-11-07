use clap::Parser;
use mrde_service::{
    backend::{PreviewExecutor, SystemExecutor},
    settings::{PreviewSettings, SettingsService},
};
use mrde_tui::{app::App, runtime::Driver};
use std::{path::PathBuf, sync::Arc};

#[derive(Parser)]
#[command(about = "MR.DE terminal review cockpit", version)]
struct Args {
    /// Use synthetic fixtures. Never reads credentials or contacts a forge.
    #[arg(long)]
    demo: bool,
    /// Select a configured project for this session.
    #[arg(long)]
    project: Option<String>,
    /// Override interface language for this session.
    #[arg(long, value_parser = ["en", "fr"])]
    language: Option<String>,
    /// Override Claude response language for this session.
    #[arg(long, value_parser = ["en", "fr"])]
    information_language: Option<String>,
    /// Export the actual terminal renderer as SVG. Requires --demo.
    #[arg(long, requires = "demo")]
    snapshot: Option<PathBuf>,
    /// Initial demo screen (also used for snapshots).
    #[arg(long, default_value = "inbox", value_parser = ["inbox","description","files","radar","review","history","projects","settings","worktrees"])]
    screen: String,
    #[arg(long, default_value_t = 140)]
    width: u16,
    #[arg(long, default_value_t = 44)]
    height: u16,
}
fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut app = if args.demo {
        App::demo()
    } else {
        let mut catalog = mrde_service::backend::load_projects()?;
        if let Some(project) = args.project {
            catalog.active = project;
        }
        let config = catalog.current()?;
        let queue = mrde_service::backend::cached_queue(&config).unwrap_or_default();
        App::new(catalog, queue, false)?
    };
    if let Some(language) = args.language {
        app.config.languages.interface =
            mrde_core::language::Language::parse(&language).expect("validated option");
    }
    if let Some(language) = args.information_language {
        app.config.languages.information =
            mrde_core::language::Language::parse(&language).expect("validated option");
    }
    app.language_draft = app.config.languages;
    if args.demo {
        mrde_tui::snapshot::scenario(&mut app, &args.screen)?;
    }
    if let Some(path) = args.snapshot {
        return mrde_tui::snapshot::export(&mut app, &path, args.width, args.height);
    }
    let driver = if args.demo {
        Driver::new(Arc::new(PreviewExecutor), Arc::new(PreviewSettings))?
    } else {
        Driver::new(
            Arc::new(SystemExecutor::default()),
            Arc::new(SettingsService::system()?),
        )?
    };
    mrde_tui::runtime::run(app, driver)
}
