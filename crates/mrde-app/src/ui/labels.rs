//! GitLab colors are authoritative; scoped values use a white adjoining segment.
use crate::theme::{FIELD, INK, OneLine};
use gpui::{Div, div, prelude::*, px, rgb};
use mrde_core::models::Label;

fn hex(value: &str) -> Option<u32> {
    let value = value.strip_prefix('#')?;
    if !value.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match value.len() {
        6 => u32::from_str_radix(value, 16).ok(),
        3 => u32::from_str_radix(&value.chars().flat_map(|c| [c, c]).collect::<String>(), 16).ok(),
        _ => None,
    }
}
fn luminance(color: u32) -> f64 {
    [(16, 0.2126), (8, 0.7152), (0, 0.0722)]
        .into_iter()
        .map(|(shift, weight)| {
            let c = ((color >> shift) & 255) as f64 / 255.;
            weight
                * if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
        })
        .sum()
}
fn contrast(a: u32, b: u32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn colors(label: &Label) -> (u32, u32, u32) {
    let background = label.color.as_deref().and_then(hex).unwrap_or(FIELD);
    let foreground = label
        .text_color
        .as_deref()
        .and_then(hex)
        .unwrap_or_else(|| {
            if label.color.as_deref().and_then(hex).is_none() {
                INK
            } else if contrast(background, 0xffffff) >= contrast(background, 0x000000) {
                0xffffff
            } else {
                0x000000
            }
        });
    // Preserve the label color on white when readable; pale labels need dark text.
    let value = if contrast(background, 0xffffff) >= 4.5 {
        background
    } else {
        0x202328
    };
    (background, foreground, value)
}
/// Priority labels (`P0`, `P::1`) first, the others in their original order.
pub(super) fn display_order<'a>(labels: impl Iterator<Item = &'a Label>) -> Vec<&'a Label> {
    let mut labels: Vec<&Label> = labels.collect();
    labels.sort_by_key(|label| mrde_core::scoring::priority([*label]) == u32::MAX);
    labels
}

pub(super) fn label_chip(label: &Label) -> Div {
    let (background, foreground, value_color) = colors(label);
    let pill = div()
        .flex()
        .flex_shrink_0()
        .rounded(px(4.))
        .overflow_hidden()
        .border_1()
        .border_color(rgb(background))
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .text_size(px(11.))
        .line_height(px(18.))
        .whitespace_nowrap();
    if let Some((scope, value)) = label.scope() {
        let name = label.name.clone();
        let value_name = name.clone();
        pill.child(
            div()
                .debug_selector(move || format!("label-scope-{name}"))
                .px(px(6.))
                .max_w(px(220.))
                .one_line()
                .child(crate::ellipsis::ellipsis(scope.to_owned())),
        )
        .child(
            div()
                .debug_selector(move || format!("label-value-{value_name}"))
                .px(px(6.))
                .max_w(px(260.))
                .one_line()
                .bg(rgb(0xffffff))
                .text_color(rgb(value_color))
                .child(crate::ellipsis::ellipsis(value.to_owned())),
        )
    } else {
        pill.child(
            div()
                .px(px(6.))
                .max_w(px(300.))
                .one_line()
                .child(crate::ellipsis::ellipsis(label.name.clone())),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gitlab_colors_are_preserved_and_white_values_remain_legible() {
        let label = Label {
            name: "Review::Dev".into(),
            color: Some("#428BCA".into()),
            text_color: Some("#FFFFFF".into()),
        };
        let (bg, fg, value) = colors(&label);
        assert_eq!((bg, fg), (0x428bca, 0xffffff));
        assert!(contrast(value, 0xffffff) >= 4.5);
        let dark = Label {
            color: Some("#123456".into()),
            ..label.clone()
        };
        assert_eq!(colors(&dark).2, 0x123456);
        let pale = Label {
            color: Some("#ff0".into()),
            text_color: None,
            ..label.clone()
        };
        assert_eq!(colors(&pale).0, 0xffff00);
        assert!(contrast(colors(&pale).1, 0xffff00) >= 4.5);
        assert!(contrast(colors(&pale).2, 0xffffff) >= 4.5);
        let invalid = Label {
            color: Some("url(secret)".into()),
            text_color: Some("#zxy".into()),
            ..label
        };
        assert_eq!(colors(&invalid), (FIELD, INK, FIELD));
    }
    #[test]
    fn scoped_labels_split_once_without_changing_their_name() {
        for (name, expected) in [
            ("Type::Feature", Some(("Type", "Feature"))),
            ("Env::a::b", Some(("Env", "a::b"))),
            ("P0", None),
            ("::value", None),
            ("scope::", None),
        ] {
            let label = Label::from(name);
            assert_eq!(label.scope(), expected);
            assert_eq!(label.name, name);
        }
    }
}
