use tower_lsp::lsp_types::{Color, ColorPresentation, Range, TextEdit};

pub(crate) fn color_summary(color: Color) -> String {
    let rgba = rgba8(color);
    let (hue, saturation, lightness) = rgb_to_hsl(color.red, color.green, color.blue);
    format!(
        "Zed Pigments\n\n```css\n{}\n{}\nhsl({} {}% {}% / {})\n```\n",
        hex(color, color.alpha < 0.9995, false, false),
        rgb(color, false, false),
        number(hue, 1),
        number(saturation * 100.0, 1),
        number(lightness * 100.0, 1),
        number(color.alpha, 3),
    )
    .replace(
        "Zed Pigments",
        &format!(
            "**Zed Pigments** · RGBA({}, {}, {}, {})",
            rgba[0],
            rgba[1],
            rgba[2],
            number(color.alpha, 3)
        ),
    )
}

pub(crate) fn color_presentations(
    color: Color,
    range: Range,
    original: Option<(&str, Color)>,
) -> Vec<ColorPresentation> {
    let mut labels = Vec::new();

    if let Some((text, original_color)) = original {
        if same_color(color, original_color) {
            labels.push(text.to_owned());
        } else if text.starts_with('#') || text.starts_with("0x") || text.starts_with("0X") {
            labels.push(hex_like(color, text));
        } else {
            let name = text
                .split_once('(')
                .map(|(name, _)| name.to_ascii_lowercase())
                .unwrap_or_default();
            match name.as_str() {
                "rgb" | "rgba" => labels.push(rgb(color, text.contains(','), text.contains('%'))),
                "hsl" | "hsla" => labels.push(hsl(color, text.contains(','))),
                _ => {}
            }
        }
    }

    labels.push(hex(color, color.alpha < 0.9995, false, false));
    labels.push(rgb(color, false, false));
    labels.push(hsl(color, false));
    labels.dedup();

    labels
        .into_iter()
        .map(|label| ColorPresentation {
            text_edit: Some(TextEdit {
                range,
                new_text: label.clone(),
            }),
            label,
            additional_text_edits: None,
        })
        .collect()
}

fn hex_like(color: Color, original: &str) -> String {
    let is_rust = original.starts_with("0x") || original.starts_with("0X");
    let prefix_length = if is_rust { 2 } else { 1 };
    let digits = original.len().saturating_sub(prefix_length);
    let include_alpha = matches!(digits, 4 | 8) || color.alpha < 0.9995;
    let short = matches!(digits, 3 | 4) && can_shorten(color, include_alpha);
    let uppercase = original.bytes().any(|byte| byte.is_ascii_uppercase());
    hex(color, include_alpha, short, uppercase).replacen(
        '#',
        if original.starts_with("0X") {
            "0X"
        } else if is_rust {
            "0x"
        } else {
            "#"
        },
        1,
    )
}

fn hex(color: Color, include_alpha: bool, short: bool, uppercase: bool) -> String {
    let rgba = rgba8(color);
    let mut digits = if include_alpha {
        format!(
            "{:02x}{:02x}{:02x}{:02x}",
            rgba[0], rgba[1], rgba[2], rgba[3]
        )
    } else {
        format!("{:02x}{:02x}{:02x}", rgba[0], rgba[1], rgba[2])
    };
    if short && can_shorten(color, include_alpha) {
        digits = digits.chars().step_by(2).collect();
    }
    if uppercase {
        digits.make_ascii_uppercase();
    }
    format!("#{digits}")
}

fn can_shorten(color: Color, include_alpha: bool) -> bool {
    rgba8(color)
        .into_iter()
        .take(if include_alpha { 4 } else { 3 })
        .all(|component| component >> 4 == component & 0x0f)
}

fn rgb(color: Color, commas: bool, percentages: bool) -> String {
    let alpha = color.alpha < 0.9995;
    let rgba = rgba8(color);
    let components = if percentages {
        [
            format!("{}%", number(color.red * 100.0, 1)),
            format!("{}%", number(color.green * 100.0, 1)),
            format!("{}%", number(color.blue * 100.0, 1)),
        ]
    } else {
        [
            rgba[0].to_string(),
            rgba[1].to_string(),
            rgba[2].to_string(),
        ]
    };

    if commas {
        let function = if alpha { "rgba" } else { "rgb" };
        let alpha = if alpha {
            format!(", {}", number(color.alpha, 3))
        } else {
            String::new()
        };
        format!(
            "{function}({}, {}, {}{alpha})",
            components[0], components[1], components[2]
        )
    } else {
        let alpha = if alpha {
            format!(" / {}", number(color.alpha, 3))
        } else {
            String::new()
        };
        format!(
            "rgb({} {} {}{alpha})",
            components[0], components[1], components[2]
        )
    }
}

fn hsl(color: Color, commas: bool) -> String {
    let (hue, saturation, lightness) = rgb_to_hsl(color.red, color.green, color.blue);
    let hue = number(hue, 1);
    let saturation = number(saturation * 100.0, 1);
    let lightness = number(lightness * 100.0, 1);
    if commas {
        if color.alpha < 0.9995 {
            format!(
                "hsla({hue}, {saturation}%, {lightness}%, {})",
                number(color.alpha, 3)
            )
        } else {
            format!("hsl({hue}, {saturation}%, {lightness}%)")
        }
    } else {
        let alpha = if color.alpha < 0.9995 {
            format!(" / {}", number(color.alpha, 3))
        } else {
            String::new()
        };
        format!("hsl({hue} {saturation}% {lightness}%{alpha})")
    }
}

fn rgba8(color: Color) -> [u8; 4] {
    [
        channel(color.red),
        channel(color.green),
        channel(color.blue),
        channel(color.alpha),
    ]
}

fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn rgb_to_hsl(red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let delta = max - min;
    let lightness = (max + min) / 2.0;
    if delta.abs() < f32::EPSILON {
        return (0.0, 0.0, lightness);
    }

    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue = if max == red {
        60.0 * ((green - blue) / delta).rem_euclid(6.0)
    } else if max == green {
        60.0 * ((blue - red) / delta + 2.0)
    } else {
        60.0 * ((red - green) / delta + 4.0)
    };
    (hue, saturation, lightness)
}

fn number(value: f32, precision: usize) -> String {
    let formatted = format!("{value:.precision$}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-0" {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn same_color(left: Color, right: Color) -> bool {
    (left.red - right.red).abs() < 0.0005
        && (left.green - right.green).abs() < 0.0005
        && (left.blue - right.blue).abs() < 0.0005
        && (left.alpha - right.alpha).abs() < 0.0005
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Position;

    fn color(red: f32, green: f32, blue: f32, alpha: f32) -> Color {
        Color {
            red,
            green,
            blue,
            alpha,
        }
    }

    #[test]
    fn preserves_hex_width_case_and_prefix_when_possible() {
        let range = Range::new(Position::new(0, 0), Position::new(0, 4));
        let labels = color_presentations(
            color(0.0, 1.0, 0.0, 1.0),
            range,
            Some(("#ABC", color(0.67, 0.73, 0.8, 1.0))),
        )
        .into_iter()
        .map(|item| item.label)
        .collect::<Vec<_>>();
        assert_eq!(labels[0], "#0F0");

        let labels = color_presentations(
            color(1.0, 0.0, 0.0, 0.5),
            range,
            Some(("0xAABBCCDD", color(0.67, 0.73, 0.8, 0.87))),
        )
        .into_iter()
        .map(|item| item.label)
        .collect::<Vec<_>>();
        assert_eq!(labels[0], "0xFF000080");
    }

    #[test]
    fn preserves_rgb_and_hsl_styles() {
        let range = Range::default();
        let rgba = color_presentations(
            color(1.0, 0.5, 0.0, 0.5),
            range,
            Some(("rgba(0, 0, 0, 1)", color(0.0, 0.0, 0.0, 1.0))),
        );
        assert_eq!(rgba[0].label, "rgba(255, 128, 0, 0.5)");

        let hsl = color_presentations(
            color(1.0, 0.0, 0.0, 1.0),
            range,
            Some(("hsl(0 0% 0%)", color(0.0, 0.0, 0.0, 1.0))),
        );
        assert_eq!(hsl[0].label, "hsl(0 100% 50%)");
    }
}
