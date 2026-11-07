//! Deterministic export of the actual Ratatui buffer, using synthetic data only.
use crate::{
    app::{App, Screen, Tab},
    ui::View,
};
use ratatui::{Terminal, backend::TestBackend, style::Color};
use std::path::Path;

pub fn scenario(app: &mut App, screen: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        app.demo,
        "Snapshots require --demo; real credentials are never read"
    );
    match screen {
        "inbox" => app.screen = Screen::Inbox,
        "settings" => app.screen = Screen::Settings,
        "projects" => app.screen = Screen::Projects,
        "worktrees" => {
            app.screen = Screen::Worktrees;
            app.worktrees = mrde_service::demo::worktrees();
        }
        "description" | "files" | "radar" | "review" | "history" => {
            let detail = mrde_service::demo::queue()
                .iter()
                .find(|mr| mr.id.iid == 231)
                .and_then(|mr| mrde_service::demo::detail(&mr.id))
                .expect("demo MR");
            app.risk = Some(Box::new(mrde_service::demo::risk(&detail)));
            app.baseline = Some(mrde_service::demo::baseline());
            app.review = Some(mrde_service::demo::review());
            app.insights = Some(Box::new(mrde_service::demo::insights(&detail)));
            app.detail = Some(Box::new(detail));
            app.screen = Screen::Reader;
            app.tab = match screen {
                "files" => Tab::Files,
                "radar" => Tab::Radar,
                "review" => Tab::Review,
                "history" => Tab::History,
                _ => Tab::Description,
            };
        }
        _ => anyhow::bail!("Unknown demo screen: {screen}"),
    }
    Ok(())
}
pub fn export(app: &mut App, path: &Path, width: u16, height: u16) -> anyhow::Result<()> {
    anyhow::ensure!(app.demo, "Snapshots require --demo");
    anyhow::ensure!(
        (80..=240).contains(&width) && (24..=100).contains(&height),
        "Snapshot dimensions must be 80..240 columns and 24..100 rows"
    );
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    View::default().draw_for_export(&mut terminal, app)?;
    let buffer = terminal.backend().buffer();
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}"><title>MR.DE terminal demo — {}</title><rect width="100%" height="100%" fill="#141518"/><g font-family="Menlo,DejaVu Sans Mono,monospace" font-size="14" xml:space="preserve">"##,
        u32::from(width) * 9 + 32,
        u32::from(height) * 20 + 32,
        u32::from(width) * 9 + 32,
        u32::from(height) * 20 + 32,
        app.config.languages.interface.code()
    );
    for y in 0..height {
        for x in 0..width {
            let cell = &buffer[(x, y)];
            let px = u32::from(x) * 9 + 16;
            let py = u32::from(y) * 20 + 16;
            svg.push_str(&format!(
                r#"<rect x="{px}" y="{py}" width="9" height="20" fill="{}"/>"#,
                color(cell.bg)
            ));
            if cell.symbol() != " " {
                svg.push_str(&format!(
                    r#"<text x="{px}" y="{}" fill="{}"{}>{}</text>"#,
                    py + 15,
                    color(cell.fg),
                    if cell.modifier.contains(ratatui::style::Modifier::BOLD) {
                        r#" font-weight="bold""#
                    } else {
                        ""
                    },
                    escape(cell.symbol())
                ));
            }
        }
    }
    svg.push_str("</g></svg>\n");
    std::fs::write(path, svg)?;
    Ok(())
}
impl View {
    fn draw_for_export(
        &mut self,
        terminal: &mut Terminal<TestBackend>,
        app: &mut App,
    ) -> anyhow::Result<()> {
        terminal.draw(|frame| self.draw(frame, app))?;
        Ok(())
    }
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn color(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::White => "#ffffff".into(),
        Color::Black => "#000000".into(),
        Color::Reset => "#eef0f4".into(),
        _ => "#bbc2ce".into(),
    }
}
