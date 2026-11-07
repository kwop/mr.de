//! Global presentation preferences, saved independently of connection credentials.
use super::*;
use mrde_core::language::Language;

impl Cockpit {
    pub(super) fn language_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.language_saving || self.is_busy() || self.initializing;
        let mut content = div().flex().flex_col().gap_3().mb_8().pb_6().border_b_1().border_color(rgb(LINE))
            .child(div().text_size(px(20.)).text_color(rgb(HEADING)).child(t("Languages")))
            .child(div().text_sm().text_color(rgb(MUTED)).child(t("Choose the interface and Claude response languages independently. Applies to all projects.")));
        for (information, title, selected) in [
            (false, "Interface language", self.language_draft.interface),
            (
                true,
                "Claude response language",
                self.language_draft.information,
            ),
        ] {
            let mut options = div().flex().gap_2();
            for language in Language::ALL {
                let id = format!(
                    "language-{}-{}",
                    if information {
                        "information"
                    } else {
                        "interface"
                    },
                    language.code()
                );
                options = options.child(
                    div()
                        .debug_selector({
                            let id = id.clone();
                            move || id.clone()
                        })
                        .child(
                            Button::new(SharedString::from(id))
                                .small()
                                .label(language.name())
                                .when(selected == language, |b| b.bg(rgb(SELECTED)))
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if information {
                                        this.language_draft.information = language;
                                    } else {
                                        this.language_draft.interface = language;
                                    }
                                    this.language_feedback.clear();
                                    cx.notify();
                                })),
                        ),
                );
            }
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(t(title))
                    .child(options),
            );
        }
        content
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div().debug_selector(|| "save-languages".into()).child(
                            Button::new("save-languages")
                                .small()
                                .label(t("Save languages"))
                                .loading(self.language_saving)
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.save_languages(cx))),
                        ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(text(&self.language_feedback)),
                    ),
            )
            .into_any_element()
    }
    fn save_languages(&mut self, cx: &mut Context<Self>) {
        if self.language_saving || self.is_busy() {
            return;
        }
        let Some(service) = self.settings_service.clone() else {
            return;
        };
        let languages = self.language_draft;
        self.language_saving = true;
        self.language_feedback = "Saving…".into();
        let task = self
            .runtime
            .spawn(async move { service.save_languages(languages).await });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.language_saving = false;
                match result {
                    Ok(Ok(())) => {
                        if this.config.languages.information != languages.information {
                            this.review = None;
                            this.review_level = None;
                            this.tour = None;
                            this.compliance = None;
                        }
                        this.config.languages = languages;
                        for project in &mut this.projects {
                            project.languages = languages;
                        }
                        this.threads
                            .update(cx, |threads, cx| threads.configure(this.config.clone(), cx));
                        crate::i18n::set(languages.interface);
                        gpui_component::set_locale(languages.interface.code());
                        this.language_feedback = "Languages saved".into();
                        this.status = "Ready".into();
                    }
                    Ok(Err(error)) => this.language_feedback = format!("{error:#}"),
                    Err(error) => this.language_feedback = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
