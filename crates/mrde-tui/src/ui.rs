//! Ratatui rendering. Only visible inbox rows are built; MR Markdown is cached.
use crate::app::*;
use mrde_core::{
    language::{Language, translate},
    models::Label,
    risk::{Axis, RiskReport},
};
use ratatui::symbols::Marker;
use ratatui::{
    prelude::*,
    widgets::{
        Block, BorderType, Borders, Clear, Paragraph, Wrap,
        canvas::{Canvas, Line as CanvasLine, Points},
    },
};

pub const BACKGROUND: Color = Color::Rgb(20, 21, 24);
pub const PANEL: Color = Color::Rgb(32, 35, 40);
pub const READER: Color = Color::Rgb(41, 44, 50);
pub const BORDER: Color = Color::Rgb(83, 89, 101);
pub const TEXT: Color = Color::Rgb(238, 240, 244);
pub const MUTED: Color = Color::Rgb(187, 194, 206);
pub const BLUE: Color = Color::Rgb(138, 180, 248);
pub const GREEN: Color = Color::Rgb(107, 204, 145);
pub const YELLOW: Color = Color::Rgb(239, 184, 93);
pub const RED: Color = Color::Rgb(240, 128, 128);
pub const SELECTED: Color = Color::Rgb(52, 68, 94);

#[derive(Default)]
pub struct View {
    markdown: Option<(String, Vec<Line<'static>>)>,
}
impl View {
    pub fn draw(&mut self, frame: &mut Frame, app: &mut App) {
        let area = frame.area();
        app.hits.clear();
        frame.render_widget(
            Block::default().style(Style::default().bg(BACKGROUND).fg(TEXT)),
            area,
        );
        if area.width < 80 || area.height < 24 {
            frame.render_widget(
                Paragraph::new(app.t("Enlarge the terminal to at least 80 × 24. Press q to quit."))
                    .wrap(Wrap { trim: false }),
                area.inner(Margin::new(2, 2)),
            );
            return;
        }
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .areas(area);
        let label = if app.demo {
            "DEMO · synthetic data"
        } else {
            "Connected workspace"
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  mr.de  ", Style::default().bold()),
                Span::styled(app.config.name.clone(), BLUE),
                Span::raw("   "),
                Span::styled(app.t(label), MUTED),
            ])),
            header,
        );
        let [nav, content] =
            Layout::horizontal([Constraint::Length(21), Constraint::Min(0)]).areas(body);
        sidebar(frame, app, nav);
        match app.screen {
            Screen::Inbox => inbox(frame, app, content),
            Screen::Reader => self.reader(frame, app, content),
            Screen::Projects => projects(frame, app, content),
            Screen::Settings => settings(frame, app, content),
            Screen::Worktrees => worktrees(frame, app, content),
        }
        let status = app.progress().unwrap_or(&app.status);
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    format!("  {}", app.t(status)),
                    if app.error { RED } else { MUTED },
                ),
                Line::styled(
                    app.t("  i Inbox   p Projects   w Worktrees   , Settings   ? Help   q Quit"),
                    MUTED,
                ),
            ]),
            footer,
        );
        if app.confirmation.is_some() {
            confirmation(frame, app, area);
        }
        if app.help {
            help(frame, app, area);
        }
    }
    fn reader(&mut self, frame: &mut Frame, app: &mut App, area: Rect) {
        let inner = panel(frame, area, app.t("Merge request"), READER);
        let Some(detail) = &app.detail else {
            frame.render_widget(Paragraph::new(app.t("Loading merge request…")), inner);
            return;
        };
        let [heading, tabs, body, actions] = Layout::vertical([
            Constraint::Length(4),
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .areas(inner);
        let title = format!(
            "{} !{} · {}",
            detail.mr.project, detail.mr.id.iid, detail.mr.author
        );
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(title, MUTED),
                Line::styled(detail.mr.title.clone(), Style::default().bold()),
                Line::from(labels(detail.mr.all_labels())),
            ])
            .wrap(Wrap { trim: false }),
            heading,
        );
        let mut x = tabs.x;
        for (index, tab) in Tab::ALL.into_iter().enumerate() {
            let title = format!("{} {}", index + 1, app.t(tab.label()));
            let width = (title.chars().count() as u16 + 2).min(tabs.right().saturating_sub(x));
            let rect = Rect::new(x, tabs.y, width, 1);
            frame.render_widget(
                Paragraph::new(format!(" {title} ")).style(
                    Style::default()
                        .fg(if app.tab == tab { BLUE } else { MUTED })
                        .bg(if app.tab == tab { SELECTED } else { READER }),
                ),
                rect,
            );
            app.hits.push((rect, Hit::Tab(tab)));
            x += width;
        }
        if app.tab == Tab::Radar {
            radar(frame, app, body);
        } else {
            let lines = match app.tab {
                Tab::Description => {
                    let source = &app.detail.as_ref().expect("detail").mr.description;
                    if self
                        .markdown
                        .as_ref()
                        .is_none_or(|(previous, _)| previous != source)
                    {
                        self.markdown = Some((source.clone(), markdown(source)));
                    }
                    let mut lines = self.markdown.as_ref().expect("markdown").1.clone();
                    for ticket in &app.detail.as_ref().expect("detail").issues {
                        lines.push(Line::raw(""));
                        lines.push(Line::styled(ticket.title.clone(), BLUE));
                        lines.push(Line::styled(ticket.web_url.clone(), MUTED));
                    }
                    lines
                }
                Tab::Files => {
                    let mut lines = vec![
                        Line::styled(app.t("Changed files").to_owned(), Style::default().bold()),
                        Line::raw(""),
                    ];
                    for file in &app.detail.as_ref().expect("detail").files {
                        let path = if file.new_path != file.old_path {
                            format!("{} → {}", file.old_path, file.new_path)
                        } else {
                            file.new_path.clone()
                        };
                        lines.push(Line::from(vec![
                            Span::styled("  • ", BLUE),
                            Span::raw(path),
                        ]));
                    }
                    lines
                }
                Tab::Review => review(app),
                Tab::History => history(app),
                Tab::Radar => unreachable!(),
            };
            scroll_text(frame, app, lines, body);
        }
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(app.t("a Approve  "), GREEN),
                    Span::raw(app.t("v Review  c Ask Claude  r Radar  m Median  h History")),
                ]),
                Line::styled(
                    format!(
                        "{}: {}  ·  {}",
                        app.t("l Review depth"),
                        app.t(app.level.label()),
                        app.t("Esc Cancel / inbox · Tab Switch tab")
                    ),
                    MUTED,
                ),
            ]),
            actions,
        );
    }
}
fn panel(frame: &mut Frame, rect: Rect, title: &str, background: Color) -> Rect {
    let block = Block::default()
        .title(Line::styled(format!(" {title} "), MUTED))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(BORDER)
        .style(Style::default().bg(background).fg(TEXT));
    let inner = block.inner(rect).inner(Margin::new(1, 0));
    frame.render_widget(block, rect);
    inner
}
fn sidebar(frame: &mut Frame, app: &mut App, rect: Rect) {
    frame.render_widget(
        Block::default()
            .style(Style::default().bg(Color::Rgb(28, 30, 34)))
            .borders(Borders::RIGHT)
            .border_style(BORDER),
        rect,
    );
    let items = [
        (Navigation::Inbox, "Inbox", Screen::Inbox),
        (Navigation::Projects, "Projects", Screen::Projects),
        (Navigation::Worktrees, "Worktrees", Screen::Worktrees),
        (Navigation::Settings, "Settings", Screen::Settings),
    ];
    for (index, (nav, title, screen)) in items.into_iter().enumerate() {
        let selected =
            app.screen == screen || (screen == Screen::Inbox && app.screen == Screen::Reader);
        let area = Rect::new(rect.x + 1, rect.y + 1 + index as u16 * 2, rect.width - 3, 1);
        frame.render_widget(
            Paragraph::new(format!(" {}", app.t(title))).style(
                Style::default()
                    .bg(if selected {
                        SELECTED
                    } else {
                        Color::Rgb(28, 30, 34)
                    })
                    .fg(if selected { TEXT } else { MUTED }),
            ),
            area,
        );
        app.hits.push((area, Hit::Nav(nav)));
    }
    let area = Rect::new(
        rect.x + 2,
        rect.bottom().saturating_sub(4),
        rect.width - 4,
        3,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(app.config.provider.label(), MUTED),
            Line::styled("SSH · local Git", MUTED),
            Line::styled(
                format!(
                    "{} / {}",
                    app.config.languages.interface.code(),
                    app.config.languages.information.code()
                ),
                MUTED,
            ),
        ]),
        area,
    );
}
fn color(value: Option<&str>, fallback: Color) -> Color {
    value
        .and_then(|v| v.strip_prefix('#'))
        .filter(|v| v.len() == 6)
        .and_then(|v| u32::from_str_radix(v, 16).ok())
        .map(|v| Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
        .unwrap_or(fallback)
}
fn labels<'a>(labels: impl Iterator<Item = &'a Label>) -> Vec<Span<'static>> {
    let mut spans = vec![];
    for label in labels {
        let background = color(label.color.as_deref(), SELECTED);
        let foreground = color(label.text_color.as_deref(), TEXT);
        if let Some((scope, value)) = label.scope() {
            spans.push(Span::styled(
                format!(" {scope} "),
                Style::default().bg(background).fg(foreground),
            ));
            spans.push(Span::styled(
                format!(" {value} "),
                Style::default().bg(Color::White).fg(Color::Black),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {} ", label.name),
                Style::default().bg(background).fg(foreground),
            ));
        }
        spans.push(Span::raw(" "));
    }
    spans
}
fn inbox(frame: &mut Frame, app: &mut App, rect: Rect) {
    let inner = panel(frame, rect, app.t("Merge requests"), PANEL);
    let [filters, list] =
        Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).areas(inner);
    let roles = mrde_core::config::MrRole::ALL
        .into_iter()
        .enumerate()
        .map(|(i, role)| {
            format!(
                "{} {}{}",
                i + 1,
                if app.config.mr_roles.contains(&role) {
                    "● "
                } else {
                    "○ "
                },
                app.t(role.label())
            )
        })
        .collect::<Vec<_>>()
        .join("   ");
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(roles, MUTED),
            Line::from(format!(
                "s {}: {}   r {}",
                app.t("Sort"),
                app.t(app.config.mr_sort.label()),
                app.t("Refresh")
            )),
            Line::styled(
                format!(
                    "/ {}: {}{}",
                    app.t("Search"),
                    app.search,
                    if app.searching { "▏" } else { "" }
                ),
                if app.searching { BLUE } else { MUTED },
            ),
        ]),
        filters,
    );
    let rows = app.filtered();
    let count = rows.len();
    if count == 0 {
        frame.render_widget(
            Paragraph::new(app.t("No merge requests match your filters."))
                .wrap(Wrap { trim: false }),
            list,
        );
        return;
    }
    let per_page = usize::from((list.height / 4).max(1));
    let first = app.selected / per_page * per_page;
    let mut hits = vec![];
    for (row, mr) in rows.iter().enumerate().skip(first).take(per_page) {
        let area = Rect::new(
            list.x,
            list.y + (row - first) as u16 * 4,
            list.width,
            4.min(
                list.bottom()
                    .saturating_sub(list.y + (row - first) as u16 * 4),
            ),
        );
        let pipeline = mr.pipeline.as_deref().unwrap_or("unknown");
        let style = match pipeline {
            "success" => GREEN,
            "failed" => RED,
            "running" | "pending" => YELLOW,
            _ => MUTED,
        };
        let age = (chrono::Utc::now() - mr.created_at).num_days();
        let info = format!(
            "!{}  {} · {} · {}d  · {} {}  · {} {}",
            mr.id.iid,
            mr.project,
            mr.author,
            age,
            mr.changed_lines
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
            app.t("lines"),
            mr.approvals
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
            app.t("approvals")
        );
        let mut title = vec![Span::styled(
            if row == app.selected { "▸ " } else { "  " },
            BLUE,
        )];
        if mr.draft {
            title.push(Span::styled(format!("[{}] ", app.t("Draft")), YELLOW));
        }
        title.push(Span::styled(mr.title.clone(), Style::default().bold()));
        let mut metadata = vec![
            Span::styled(format!("  {info}   "), MUTED),
            Span::styled(format!("● {}", app.t(pipeline)), style),
        ];
        if let Some(risk) = &mr.risk {
            metadata.push(Span::styled(
                format!(
                    "  {} {}",
                    app.t("Risk"),
                    risk.mean
                        .map(|v| format!("{v:.1}/10"))
                        .unwrap_or_else(|| "n/a".into())
                ),
                BLUE,
            ));
        }
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(title),
                Line::from(metadata),
                Line::from(labels(mr.all_labels())),
                Line::raw(""),
            ])
            .style(Style::default().bg(if row == app.selected {
                SELECTED
            } else {
                PANEL
            })),
            area,
        );
        hits.push((area, Hit::Row(row)));
    }
    app.hits.extend(hits);
}
fn scroll_text(frame: &mut Frame, app: &mut App, lines: Vec<Line<'static>>, area: Rect) {
    let widget = Paragraph::new(lines).wrap(Wrap { trim: false });
    // Clamp against wrapped lines, not source lines, so End always reveals the last line.
    let max = widget
        .line_count(area.width)
        .saturating_sub(usize::from(area.height))
        .min(usize::from(u16::MAX));
    app.scroll = app.scroll.min(max as u16);
    frame.render_widget(widget.scroll((app.scroll, 0)), area);
}
fn review(app: &App) -> Vec<Line<'static>> {
    let Some(review) = &app.review else {
        return vec![Line::raw(
            app.t("Press v to run a review. Claude only starts after your action.")
                .to_owned(),
        )];
    };
    let mut lines = vec![
        Line::styled(app.t("Summary").to_owned(), Style::default().bold()),
        Line::raw(review.summary.clone()),
        Line::raw(""),
        Line::styled(app.t("Intention").to_owned(), BLUE),
        Line::raw(review.intention.clone()),
        Line::raw(""),
    ];
    for risk in &review.risks {
        lines.push(Line::styled(
            format!(
                "{:?} · {}",
                risk.severity,
                risk.file.as_deref().unwrap_or("")
            ),
            YELLOW,
        ));
        lines.push(Line::raw(risk.description.clone()));
        lines.push(Line::raw(""));
    }
    lines.push(Line::styled(
        format!("{}: {:.0}%", app.t("Confidence"), review.confidence * 100.),
        MUTED,
    ));
    lines
}
fn history(app: &App) -> Vec<Line<'static>> {
    let Some(insights) = &app.insights else {
        return vec![Line::raw(
            app.t("Press h to analyze file history and ownership.")
                .to_owned(),
        )];
    };
    let mut lines = vec![
        Line::styled(app.t("File history").to_owned(), Style::default().bold()),
        Line::raw(""),
    ];
    for file in &insights.history {
        lines.push(Line::styled(file.path.clone(), BLUE));
        lines.push(Line::raw(format!(
            "  {} {} · {} {} · {} {} · {:?}",
            file.commits,
            app.t("commits"),
            file.fixes,
            app.t("fixes"),
            file.authors,
            app.t("authors"),
            file.risk
        )));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        app.t("Authors").to_owned(),
        Style::default().bold(),
    ));
    for expert in &insights.experts {
        lines.push(Line::styled(expert.zone.clone(), BLUE));
        for (author, share) in &expert.people {
            lines.push(Line::raw(format!("  {author} · {share}%")));
        }
    }
    for warning in &insights.warnings {
        lines.push(Line::styled(warning.clone(), YELLOW));
    }
    lines
}
fn projects(frame: &mut Frame, app: &mut App, rect: Rect) {
    let area = panel(frame, rect, app.t("Projects"), READER);
    for (index, project) in app.catalog.projects.iter().enumerate() {
        let y = area.y + 1 + index as u16 * 3;
        if y + 2 > area.bottom() {
            break;
        }
        let row = Rect::new(area.x, y, area.width, 2);
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    format!(
                        "{} {} · {}",
                        if project.id == app.config.id {
                            "●"
                        } else {
                            "○"
                        },
                        project.name,
                        project.provider.label()
                    ),
                    BLUE,
                ),
                Line::styled(
                    format!("  {}  {}", project.gitlab_url, project.group),
                    MUTED,
                ),
            ])
            .style(Style::default().bg(if app.project_selected == index {
                SELECTED
            } else {
                READER
            })),
            row,
        );
        app.hits.push((row, Hit::Project(index)));
    }
}
fn settings(frame: &mut Frame, app: &mut App, rect: Rect) {
    let inner = panel(frame, rect, app.t("Languages"), READER);
    let [intro, interface, information, save, hint] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(0),
    ])
    .areas(inner);
    frame.render_widget(Paragraph::new(app.t("Choose the interface and Claude response languages independently. Applies to all projects.")).wrap(Wrap { trim:false }), intro);
    for (row, key, title, language, is_information) in [
        (
            interface,
            "1",
            "Interface language",
            app.language_draft.interface,
            false,
        ),
        (
            information,
            "2",
            "Claude response language",
            app.language_draft.information,
            true,
        ),
    ] {
        frame.render_widget(
            Paragraph::new(format!("{key}  {}: {}", app.t(title), language.name())).style(BLUE),
            row,
        );
        let next = if language == Language::English {
            Language::French
        } else {
            Language::English
        };
        app.hits.push((
            row,
            if is_information {
                Hit::Information(next)
            } else {
                Hit::Interface(next)
            },
        ));
    }
    frame.render_widget(Paragraph::new(app.t("s Save languages")).style(GREEN), save);
    app.hits.push((save, Hit::SaveLanguages));
    frame.render_widget(Paragraph::new(app.t("Connections are shared with the desktop. Edit projects.toml or use native settings for credentials.")).wrap(Wrap { trim:false }), hint);
}

fn worktrees(frame: &mut Frame, app: &mut App, rect: Rect) {
    let area = panel(frame, rect, app.t("Worktrees"), READER);
    let mut lines = vec![Line::raw("")];
    for entry in &app.worktrees {
        lines.push(Line::styled(
            format!(
                "!{} · {}",
                entry.iid,
                entry.sha.chars().take(10).collect::<String>()
            ),
            BLUE,
        ));
        lines.push(Line::raw(entry.path.display().to_string()));
        lines.push(Line::raw(""));
    }
    if app.worktrees.is_empty() {
        lines.push(Line::raw(app.t("No local worktrees.").to_owned()));
    }
    scroll_text(frame, app, lines, area);
}
fn radar(frame: &mut Frame, app: &mut App, rect: Rect) {
    let Some(report) = app.risk.clone() else {
        frame.render_widget(
            Paragraph::new(
                app.t("Press r to calculate risks locally. Unavailable axes remain n/a."),
            )
            .wrap(Wrap { trim: false }),
            rect,
        );
        return;
    };
    let language = app.config.languages.interface;
    if rect.height < 20 || rect.width < 85 {
        let mut lines = vec![Line::styled(
            format!("{}: {} / 10", app.t("Mean"), score(report.profile.mean())),
            BLUE,
        )];
        for (index, axis) in report.profile.axes.iter().enumerate() {
            let median = app.baseline.as_ref().and_then(|b| b.medians[index]);
            lines.push(Line::styled(
                format!(
                    "{} {}: {}  · {} {}",
                    if index == app.axis { "▸" } else { " " },
                    app.t(axis.axis.label()),
                    score(axis.score),
                    app.t("med"),
                    score(median)
                ),
                if axis.score.is_none() { YELLOW } else { TEXT },
            ));
        }
        lines.push(Line::raw(""));
        lines.extend(axis_evidence(language, &report, app.axis));
        lines.push(Line::styled(
            app.t("Enlarge the terminal for the chart. All measurements remain available here.")
                .to_owned(),
            MUTED,
        ));
        scroll_text(frame, app, lines, rect);
        return;
    }
    let [summary, chart, evidence] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(if rect.height >= 24 { 16 } else { 10 }),
        Constraint::Min(0),
    ])
    .areas(rect);
    let verdict = report
        .profile
        .verdict(&report.verdict_thresholds)
        .map(|v| app.t(v.label()))
        .unwrap_or("n/a");
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("◆ {verdict}   "), YELLOW),
            Span::styled(
                format!(
                    "{} {} / 10  ·  {}/{} {}",
                    app.t("Mean"),
                    score(report.profile.mean()),
                    report.profile.available(),
                    Axis::COUNT,
                    app.t("axes measured")
                ),
                BLUE,
            ),
        ])),
        summary,
    );
    let [plot, axes] =
        Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).areas(chart);
    let medians = app.baseline.as_ref().map(|b| b.medians);
    let canvas = Canvas::default()
        .x_bounds([-12., 12.])
        .y_bounds([-12., 12.])
        .background_color(READER)
        .marker(Marker::Braille)
        .paint(|ctx| {
            for step in [2., 4., 6., 8., 10.] {
                for i in 0..Axis::COUNT {
                    let a = point(i, step);
                    let b = point((i + 1) % Axis::COUNT, step);
                    ctx.draw(&CanvasLine {
                        x1: a.0,
                        y1: a.1,
                        x2: b.0,
                        y2: b.1,
                        color: BORDER,
                    });
                }
            }
            for i in 0..Axis::COUNT {
                let end = point(i, 10.);
                ctx.draw(&CanvasLine {
                    x1: 0.,
                    y1: 0.,
                    x2: end.0,
                    y2: end.1,
                    color: BORDER,
                });
                let label = point(i, 11.);
                ctx.print(label.0, label.1, Line::styled((i + 1).to_string(), MUTED));
                let next = (i + 1) % Axis::COUNT;
                if let Some(medians) = medians
                    && let (Some(a), Some(b)) = (medians[i], medians[next])
                {
                    let a = point(i, a);
                    let b = point(next, b);
                    for dash in (0..12).step_by(2) {
                        let t = dash as f64 / 12.;
                        let u = (dash + 1) as f64 / 12.;
                        ctx.draw(&CanvasLine {
                            x1: a.0 + (b.0 - a.0) * t,
                            y1: a.1 + (b.1 - a.1) * t,
                            x2: a.0 + (b.0 - a.0) * u,
                            y2: a.1 + (b.1 - a.1) * u,
                            color: MUTED,
                        });
                    }
                }
                if let Some(a) = report.profile.axes[i].score {
                    let p = point(i, a);
                    ctx.draw(&Points {
                        coords: &[p],
                        color: BLUE,
                    });
                    if let Some(b) = report.profile.axes[next].score {
                        let q = point(next, b);
                        ctx.draw(&CanvasLine {
                            x1: p.0,
                            y1: p.1,
                            x2: q.0,
                            y2: q.1,
                            color: BLUE,
                        });
                    }
                }
            }
        });
    frame.render_widget(canvas, plot);
    let mut lines = vec![];
    for (i, axis) in report.profile.axes.iter().enumerate() {
        lines.push(Line::styled(
            format!(
                "{} {} {:<19} {:>4}  {} {:>4}",
                if app.axis == i { "▸" } else { " " },
                i + 1,
                translate(language, axis.axis.label()),
                score(axis.score),
                app.t("med"),
                score(medians.and_then(|m| m[i]))
            ),
            Style::default()
                .fg(if axis.score.is_none() { YELLOW } else { TEXT })
                .bg(if app.axis == i { SELECTED } else { READER }),
        ));
        if i < usize::from(axes.height) {
            app.hits.push((
                Rect::new(axes.x, axes.y + i as u16, axes.width, 1),
                Hit::Axis(i),
            ));
        }
    }
    frame.render_widget(Paragraph::new(lines), axes);
    let mut lines = axis_evidence(language, &report, app.axis);
    if let Some(baseline) = &app.baseline {
        lines.push(Line::styled(
            format!(
                "{}: {} · {} {}",
                app.t("Median saved"),
                baseline.computed_at.format("%Y-%m-%d %H:%M"),
                baseline.merged_mrs,
                app.t("merged requests")
            ),
            MUTED,
        ));
    }
    lines.push(Line::styled(
        app.t("← → Select axis · c Ask Claude · t Prepare tools · y Authorize proposed lint")
            .to_owned(),
        MUTED,
    ));
    for command in &report.pending_commands {
        lines.push(Line::styled(
            format!(
                "{}: {} ({})",
                app.t("Proposed command"),
                command.command.join(" "),
                command.directory
            ),
            YELLOW,
        ));
    }
    for message in &report.setup_messages {
        lines.push(Line::styled(message.clone(), MUTED));
    }
    scroll_text(frame, app, lines, evidence);
}
fn point(index: usize, value: f64) -> (f64, f64) {
    let angle =
        std::f64::consts::FRAC_PI_2 - index as f64 * std::f64::consts::TAU / Axis::COUNT as f64;
    (value * angle.cos(), value * angle.sin())
}
fn score(value: Option<f64>) -> String {
    value
        .map(|v| format!("{v:.1}"))
        .unwrap_or_else(|| "n/a".into())
}
fn axis_evidence(language: Language, report: &RiskReport, index: usize) -> Vec<Line<'static>> {
    let axis = &report.profile.axes[index];
    let mut lines = vec![Line::styled(
        translate(language, axis.axis.label()).to_owned(),
        Style::default().fg(BLUE).bold(),
    )];
    if let Some(raw) = &axis.raw {
        lines.push(Line::raw(format!(
            "{} {} → {} / 10",
            raw.value,
            raw.unit,
            score(axis.score)
        )));
        lines.push(Line::styled(raw.method.clone(), MUTED));
        for evidence in &raw.evidence {
            lines.push(Line::raw(format!(
                "{}{}  {}",
                evidence.path,
                evidence.line.map(|l| format!(":{l}")).unwrap_or_default(),
                evidence.detail
            )));
        }
    } else {
        lines.push(Line::styled(
            format!(
                "n/a · {}",
                axis.unavailable.as_deref().unwrap_or("Unavailable")
            ),
            YELLOW,
        ));
    }
    lines
}
fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height.saturating_sub(4));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}
fn confirmation(frame: &mut Frame, app: &App, area: Rect) {
    let area = popup(area, 82, 14);
    frame.render_widget(Clear, area);
    let inner = panel(frame, area, app.t("Confirmation"), READER);
    let Some(confirm) = &app.confirmation else {
        return;
    };
    let (title,detail,extra)=match confirm{
        Confirmation::Approve(d)=>("Approve this commit?",d,format!("{}\n{}",d.mr.web_url,d.mr.sha)),
        Confirmation::Tools(d)=>("Prepare risk tools?",d,app.t("Downloads and dependency installation may access the network and run package setup commands. Repository lint commands still require approval.").to_owned()),
        Confirmation::Lint(d,key)=>("Authorize this lint command?",d,app.risk.as_ref().and_then(|r|r.pending_commands.iter().find(|c|c.key==*key)).map(|c|format!("{}\n{}\n{}",c.directory,c.command.join(" "),c.preparation.as_deref().unwrap_or(""))).unwrap_or_default()),
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(app.t(title).to_owned(), YELLOW),
            Line::raw(""),
            Line::raw(detail.mr.title.clone()),
            Line::raw(extra),
            Line::raw(""),
            Line::styled(app.t("y Confirm · n / Esc Cancel").to_owned(), GREEN),
        ])
        .wrap(Wrap { trim: false }),
        inner,
    );
}
fn help(frame: &mut Frame, app: &App, area: Rect) {
    let area = popup(area, 85, 19);
    frame.render_widget(Clear, area);
    let inner = panel(frame, area, app.t("Keyboard shortcuts"), READER);
    let lines = [
        "i Inbox · p Projects · w Worktrees · , Settings",
        "↑ ↓ / j k Move · Enter Open · Esc Back / cancel",
        "/ Search titles, authors and labels · s Cycle sort",
        "1 2 3 Toggle reviewer, assignee and author filters",
        "Reader: 1–5 / Tab Switch tab · PgUp PgDn Scroll",
        "r Calculate radar · m Update cached team median",
        "← → Select risk axis · c Discuss this axis with Claude",
        "v Run review · l Choose depth · h Analyze history",
        "a Approve (confirmation required)",
        "t Prepare tools · y Authorize proposed lint (confirmation)",
        "Settings: 1 Interface · 2 Claude replies · s Save",
        "q / Ctrl+C Quit · Mouse click and wheel supported",
    ];
    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .map(|s| Line::raw(app.t(s).to_owned()))
                .collect::<Vec<_>>(),
        )
        .wrap(Wrap { trim: false }),
        inner,
    );
}
/// Render Markdown structure without a webview, escaping terminal control characters.
fn markdown(source: &str) -> Vec<Line<'static>> {
    let source: String = source
        .chars()
        .take(200_000)
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    let Ok(node) = markdown::to_mdast(&source, &markdown::ParseOptions::gfm()) else {
        return source.lines().map(|s| Line::raw(s.to_owned())).collect();
    };
    fn visit(node: &markdown::mdast::Node, lines: &mut Vec<Line<'static>>) {
        use markdown::mdast::Node;
        match node {
            Node::Heading(h) => {
                lines.push(Line::styled(
                    node.to_string(),
                    Style::default().fg(BLUE).bold(),
                ));
                if h.depth < 3 {
                    lines.push(Line::raw(""));
                }
            }
            Node::Paragraph(_) => {
                lines.push(Line::raw(node.to_string()));
                lines.push(Line::raw(""));
            }
            Node::Code(c) => {
                lines.push(Line::styled(c.lang.clone().unwrap_or_default(), MUTED));
                for line in c.value.lines() {
                    lines.push(Line::styled(
                        format!("  {line}"),
                        Style::default().fg(BLUE).bg(PANEL),
                    ));
                }
                lines.push(Line::raw(""));
            }
            Node::ListItem(_) => {
                lines.push(Line::raw(format!("  • {}", node.to_string())));
            }
            Node::TableRow(row) => {
                lines.push(Line::raw(
                    row.children
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("  │  "),
                ));
            }
            Node::ThematicBreak(_) => {
                lines.push(Line::styled("────────────────────────────", BORDER))
            }
            _ => {
                if let Some(children) = node.children() {
                    for child in children {
                        visit(child, lines);
                    }
                }
            }
        }
    }
    let mut lines = vec![];
    visit(&node, &mut lines);
    lines
}
