use super::*;
use gpui_component::text::{TextView, TextViewStyle};

impl Cockpit {
    fn debug_text(&self) -> String {
        self.logs
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(super) fn debug_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // A fence longer than any log backtick run preserves every character literally.
        let text = self.debug_text();
        let fence = "`".repeat(
            text.split(|c| c != '`')
                .map(str::len)
                .max()
                .unwrap_or(0)
                .max(2)
                + 1,
        );
        let source = format!("{fence}\n{text}\n{fence}");
        let style = TextViewStyle {
            code_block: StyleRefinement::default()
                .p_0()
                .m_0()
                .bg(rgb(QUEUE))
                .font_family(CODE_FONT)
                .text_size(px(11.))
                .line_height(px(18.)),
            ..TextViewStyle::default()
        };
        div()
            .debug_selector(|| "debug-panel".into())
            .h(px(168.))
            .flex_shrink_0()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(rgb(QUEUE))
            .rounded(px(RADIUS_SHEET))
            .overflow_hidden()
            .px_5()
            .py_2()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .flex_shrink_0()
                    .child(div().text_xs().text_color(rgb(MUTED)).child("Debug logs"))
                    .child(
                        div().debug_selector(|| "copy-debug-logs".into()).child(
                            Button::new("copy-debug-logs")
                                .ghost()
                                .small()
                                .label(if self.logs_copied {
                                    "Copied"
                                } else {
                                    "Copy logs"
                                })
                                .icon(if self.logs_copied {
                                    IconName::Check
                                } else {
                                    IconName::Copy
                                })
                                .disabled(self.logs.is_empty())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        this.debug_text(),
                                    ));
                                    this.logs_copied = true;
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .child(
                div()
                    .id("debug-log")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .debug_selector(|| "debug-log-text".into())
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                TextView::markdown("debug-log-content", source, window, cx)
                                    .style(style)
                                    .selectable(true)
                                    .h_auto()
                                    .flex_shrink_0()
                                    .min_w_0()
                                    .font_family(CODE_FONT)
                                    .text_size(px(11.))
                                    .text_color(rgb(MUTED))
                                    .line_height(px(18.)),
                            ),
                    ),
            )
            .into_any_element()
    }
}
