use std::collections::{HashMap, HashSet};

use csscolorparser::{Color, ParseColorError};
use tower_lsp::lsp_types::{Position, Range};

const COLOR_FUNCTIONS: &[&str] = &[
    "hsl", "hsla", "hsv", "hsva", "hwb", "hwba", "lab", "lch", "oklab", "oklch", "rgb", "rgba",
];
const MAX_VARIABLE_DEPTH: usize = 32;

#[derive(Debug, Clone)]
pub struct ColorNode {
    pub color: Color,
    pub matched: String,
    pub range: Range,
}

impl Eq for ColorNode {}

impl PartialEq for ColorNode {
    fn eq(&self, other: &Self) -> bool {
        self.matched == other.matched
            && self.range == other.range
            && self.color.to_rgba8() == other.color.to_rgba8()
    }
}

impl ColorNode {
    pub fn lsp_color(&self) -> tower_lsp::lsp_types::Color {
        tower_lsp::lsp_types::Color {
            red: self.color.r.clamp(0.0, 1.0),
            green: self.color.g.clamp(0.0, 1.0),
            blue: self.color.b.clamp(0.0, 1.0),
            alpha: self.color.a.clamp(0.0, 1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Span {
    start: usize,
    end: usize,
}

#[derive(Debug)]
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(offset + 1);
            }
        }
        Self { starts }
    }
}

struct SourceIndex<'a> {
    text: &'a str,
    lines: LineIndex,
}

impl<'a> SourceIndex<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            lines: LineIndex::new(text),
        }
    }

    fn position(&self, byte_offset: usize) -> Position {
        let line = self
            .lines
            .starts
            .partition_point(|start| *start <= byte_offset)
            - 1;
        let line_start = self.lines.starts[line];
        let character = self.text[line_start..byte_offset].encode_utf16().count();
        Position::new(line as u32, character as u32)
    }

    fn range(&self, span: Span) -> Range {
        Range::new(self.position(span.start), self.position(span.end))
    }
}

#[derive(Clone, Debug)]
struct VariableDefinition {
    value: String,
}

#[derive(Debug)]
struct VariableDefinitions {
    values: HashMap<String, VariableDefinition>,
    declaration_spans: HashSet<Span>,
    local_overrides: Vec<LocalOverride>,
}

#[derive(Debug)]
struct LocalOverride {
    name: String,
    scope: Span,
}

#[derive(Clone, Copy)]
struct ParseContext {
    stylesheet: bool,
    markup: bool,
    markdown: bool,
}

impl ParseContext {
    fn for_language(language_id: &str) -> Self {
        let language_id = language_id.to_ascii_lowercase();
        Self {
            stylesheet: matches!(
                language_id.as_str(),
                "css" | "scss" | "less" | "sass" | "stylus" | "tera (css)"
            ),
            markup: matches!(
                language_id.as_str(),
                "html"
                    | "xml"
                    | "svg"
                    | "erb"
                    | "astro"
                    | "svelte"
                    | "vue"
                    | "vue.js"
                    | "php"
                    | "tera"
                    | "tera (html)"
            ),
            markdown: matches!(language_id.as_str(), "markdown" | "md"),
        }
    }
}

pub fn try_parse_color(value: &str) -> Result<Color, ParseColorError> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        return csscolorparser::parse(&format!("#{hex}"));
    }

    if let Ok(color) = try_parse_gpui_color(value) {
        return Ok(color);
    }

    if value
        .bytes()
        .any(|byte| matches!(byte, b'\n' | b'\r' | b'\t'))
    {
        let normalized = value.split_ascii_whitespace().collect::<Vec<_>>().join(" ");
        csscolorparser::parse(&normalized)
    } else {
        csscolorparser::parse(value)
    }
}

/// Parse GPUI's normalized rgb/hsl forms, whose components are all in the 0..=1 range.
fn try_parse_gpui_color(value: &str) -> Result<Color, ParseColorError> {
    fn normalized(value: &str) -> Option<f32> {
        value
            .parse()
            .ok()
            .filter(|value| (0.0..=1.0).contains(value))
    }

    let value = value.trim();
    let (name, parameters) = value
        .split_once('(')
        .and_then(|(name, rest)| rest.strip_suffix(')').map(|rest| (name.trim(), rest)))
        .ok_or(ParseColorError::InvalidFunction)?;

    if parameters.contains('%') || parameters.contains('/') {
        return Err(ParseColorError::InvalidFunction);
    }

    let raw_values = parameters
        .split(',')
        .flat_map(str::split_ascii_whitespace)
        .collect::<Vec<_>>();
    // A trailing decimal point is accepted by Rust/GPUI but is not a CSS number.
    // Without this marker, values such as rgb(0, 0, 1) must retain CSS's 0..255
    // interpretation instead of becoming normalized bright blue.
    if !raw_values.iter().take(3).any(|value| value.ends_with('.')) {
        return Err(ParseColorError::InvalidFunction);
    }

    let values = raw_values
        .into_iter()
        .map(normalized)
        .collect::<Option<Vec<_>>>()
        .ok_or(ParseColorError::InvalidFunction)?;

    if !(3..=4).contains(&values.len()) {
        return Err(ParseColorError::InvalidFunction);
    }

    let alpha = values.get(3).copied().unwrap_or(1.0);
    if name.eq_ignore_ascii_case("rgb") || name.eq_ignore_ascii_case("rgba") {
        Ok(Color::new(values[0], values[1], values[2], alpha))
    } else if name.eq_ignore_ascii_case("hsl") || name.eq_ignore_ascii_case("hsla") {
        Ok(Color::from_hsla(
            values[0] * 360.0,
            values[1],
            values[2],
            alpha,
        ))
    } else {
        Err(ParseColorError::InvalidFunction)
    }
}

/// Parse all directly written colors and safely resolvable document-local variable references.
pub fn parse(text: &str) -> Vec<ColorNode> {
    parse_document(text, "css")
}

pub fn parse_document(text: &str, language_id: &str) -> Vec<ColorNode> {
    let index = SourceIndex::new(text);
    let context = ParseContext::for_language(language_id);
    let mut nodes = scan_direct(text, &index, context);
    let definitions = collect_definitions(text, context);
    let resolved = resolve_definitions(&definitions.values);

    for (span, name) in scan_variable_references(text, &definitions.values, context) {
        if definitions.declaration_spans.contains(&span)
            || definitions
                .local_overrides
                .iter()
                .any(|local| local.name == name && span_within(span, local.scope))
        {
            continue;
        }
        if let Some(color) = resolved.get(&name) {
            let range = index.range(span);
            nodes.retain(|node| !range_contains(range, node.range));
            // A bare Stylus variable can also spell a CSS named color. The local
            // assignment is more specific than the named-color interpretation.
            nodes.push(ColorNode {
                color: color.clone(),
                matched: text[span.start..span.end].to_owned(),
                range,
            });
        }
    }

    nodes.sort_by_key(|node| {
        (
            node.range.start.line,
            node.range.start.character,
            node.range.end.line,
            node.range.end.character,
        )
    });
    nodes.dedup_by(|left, right| left.range == right.range);
    nodes
}

fn scan_direct(text: &str, index: &SourceIndex<'_>, context: ParseContext) -> Vec<ColorNode> {
    let bytes = text.as_bytes();
    let mut nodes = Vec::new();
    let mut offset = 0;

    while offset < bytes.len() {
        let byte = bytes[offset];

        if byte == b'#' {
            if let Some(span) = scan_hex(text, offset, 1)
                .filter(|span| !excluded_hex_fragment(text, *span, context))
            {
                if let Ok(color) = try_parse_color(&text[span.start..span.end]) {
                    nodes.push(ColorNode {
                        color,
                        matched: text[span.start..span.end].to_owned(),
                        range: index.range(span),
                    });
                    offset = span.end;
                    continue;
                }
            }
        } else if byte == b'0'
            && matches!(bytes.get(offset + 1), Some(b'x' | b'X'))
            && token_boundary_before(bytes, offset)
        {
            if let Some(span) = scan_hex(text, offset, 2) {
                if let Ok(color) = try_parse_color(&text[span.start..span.end]) {
                    nodes.push(ColorNode {
                        color,
                        matched: text[span.start..span.end].to_owned(),
                        range: index.range(span),
                    });
                    offset = span.end;
                    continue;
                }
            }
        } else if byte.is_ascii_alphabetic() && token_boundary_before(bytes, offset) {
            let end = take_while(bytes, offset, u8::is_ascii_alphabetic);
            let name = &text[offset..end];

            if bytes.get(end) == Some(&b'(')
                && COLOR_FUNCTIONS
                    .iter()
                    .any(|function| name.eq_ignore_ascii_case(function))
            {
                if let Some(close) = matching_parenthesis(text, end) {
                    let span = Span {
                        start: offset,
                        end: close + 1,
                    };
                    if let Ok(color) = try_parse_color(&text[span.start..span.end]) {
                        nodes.push(ColorNode {
                            color,
                            matched: text[span.start..span.end].to_owned(),
                            range: index.range(span),
                        });
                        offset = span.end;
                        continue;
                    }
                }
            } else if token_boundary_after(bytes, end)
                && !context.markdown
                && named_color_context(text, offset, end, context)
                && name.bytes().all(|byte| byte.is_ascii_alphabetic())
            {
                if let Ok(color) = csscolorparser::parse(name) {
                    let span = Span { start: offset, end };
                    nodes.push(ColorNode {
                        color,
                        matched: name.to_owned(),
                        range: index.range(span),
                    });
                    offset = end;
                    continue;
                }
            }
        }

        offset += text[offset..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(1);
    }

    nodes
}

fn scan_hex(text: &str, start: usize, prefix_len: usize) -> Option<Span> {
    let bytes = text.as_bytes();
    if !token_boundary_before(bytes, start) {
        return None;
    }

    let digits_start = start + prefix_len;
    let digits_end = take_while(bytes, digits_start, u8::is_ascii_hexdigit);
    let digits = digits_end - digits_start;
    if !matches!(digits, 3 | 4 | 6 | 8) || !token_boundary_after(bytes, digits_end) {
        return None;
    }

    Some(Span {
        start,
        end: digits_end,
    })
}

fn matching_parenthesis(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut quote = None;
    let mut escaped = false;

    for (relative, character) in text[open..].char_indices() {
        let absolute = open + relative;
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == '(' {
            depth += 1;
        } else if character == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(absolute);
            }
        }
    }
    None
}

fn named_color_context(text: &str, start: usize, end: usize, context: ParseContext) -> bool {
    let bytes = text.as_bytes();
    let previous = previous_non_whitespace(bytes, start);
    let next = next_non_whitespace(bytes, end);

    if matches!(next.map(|(_, byte)| byte), Some(b':' | b'=' | b'('))
        || matches!(previous.map(|(_, byte)| byte), Some(b'.' | b'#' | b'-'))
    {
        return false;
    }

    if let (Some((_, before @ (b'\'' | b'"'))), Some((_, after))) = (previous, next) {
        if before == after {
            let after_quote =
                next.and_then(|(quote_offset, _)| next_non_whitespace(bytes, quote_offset + 1));
            return !matches!(after_quote.map(|(_, byte)| byte), Some(b':'));
        }
    }

    if matches!(previous.map(|(_, byte)| byte), Some(b'(' | b','))
        && matches!(next.map(|(_, byte)| byte), Some(b')' | b',' | b';'))
    {
        return true;
    }

    let line_start = text[..start].rfind('\n').map_or(0, |offset| offset + 1);
    let declaration_start = text[line_start..start]
        .rfind([';', '{', '}'])
        .map_or(line_start, |offset| line_start + offset + 1);
    has_color_assignment_context(
        text,
        declaration_start,
        start,
        end,
        is_stylesheet_at(text, start, context),
    )
}

fn collect_definitions(text: &str, context: ParseContext) -> VariableDefinitions {
    let bytes = text.as_bytes();
    let mut definitions = HashMap::new();
    let mut declaration_spans = HashSet::new();
    let mut local_overrides = Vec::new();
    let mut ambiguous = HashSet::new();
    let mut offset = 0;

    while offset < bytes.len() {
        let start = offset;
        let (name_end, allowed_colon) = if bytes[offset] == b'-'
            && bytes.get(offset + 1) == Some(&b'-')
            && token_boundary_before(bytes, offset)
        {
            (take_while(bytes, offset + 2, is_variable_character), true)
        } else if matches!(bytes[offset], b'$' | b'@') && token_boundary_before(bytes, offset) {
            (take_while(bytes, offset + 1, is_variable_character), true)
        } else if (bytes[offset].is_ascii_alphabetic() || bytes[offset] == b'_')
            && token_boundary_before(bytes, offset)
        {
            (take_while(bytes, offset + 1, is_variable_character), false)
        } else {
            offset += text[offset..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
            continue;
        };

        if name_end == start
            || (matches!(bytes[start], b'-' | b'$' | b'@') && name_end <= start + 1)
        {
            offset += 1;
            continue;
        }

        let delimiter_offset = skip_ascii_whitespace(bytes, name_end);
        let delimiter = bytes.get(delimiter_offset).copied();
        if delimiter != Some(b'=') && !(allowed_colon && delimiter == Some(b':')) {
            offset = name_end;
            continue;
        }
        let name = text[start..name_end].to_owned();
        let name_span = Span {
            start,
            end: name_end,
        };
        declaration_spans.insert(name_span);
        if bytes[start] == b'-' && !is_stylesheet_at(text, start, context) {
            offset = name_end;
            continue;
        }
        let local_scope = (bytes[start] == b'-')
            .then(|| css_custom_property_scope(text, start))
            .flatten();

        let value_start = skip_ascii_whitespace(bytes, delimiter_offset + 1);
        let value_end = text[value_start..]
            .find([';', '\n', '\r', '}'])
            .map_or(text.len(), |relative| value_start + relative);
        let value = text[value_start..value_end].trim();
        if let Some(scope) = local_scope {
            local_overrides.push(LocalOverride { name, scope });
            offset = value_end.max(name_end);
            continue;
        }
        if !value.is_empty() {
            let definition = VariableDefinition {
                value: value.to_owned(),
            };
            if !ambiguous.contains(&name) && definitions.insert(name.clone(), definition).is_some()
            {
                definitions.remove(&name);
                ambiguous.insert(name);
            }
        }
        offset = value_end.max(name_end);
    }

    VariableDefinitions {
        values: definitions,
        declaration_spans,
        local_overrides,
    }
}

fn resolve_definitions(
    definitions: &HashMap<String, VariableDefinition>,
) -> HashMap<String, Color> {
    let mut resolved = HashMap::new();
    for name in definitions.keys() {
        let mut visiting = HashSet::new();
        if let Some(color) = resolve_variable(name, definitions, &mut resolved, &mut visiting, 0) {
            resolved.insert(name.clone(), color);
        }
    }
    resolved
}

fn resolve_variable(
    name: &str,
    definitions: &HashMap<String, VariableDefinition>,
    resolved: &mut HashMap<String, Color>,
    visiting: &mut HashSet<String>,
    depth: usize,
) -> Option<Color> {
    if let Some(color) = resolved.get(name) {
        return Some(color.clone());
    }
    if depth >= MAX_VARIABLE_DEPTH || !visiting.insert(name.to_owned()) {
        return None;
    }

    let value = definitions.get(name)?.value.trim();
    let value = value
        .strip_suffix("!default")
        .or_else(|| value.strip_suffix("!global"))
        .unwrap_or(value)
        .trim();
    let color = try_parse_color(value).ok().or_else(|| {
        parse_exact_reference(value, definitions).and_then(|reference| {
            resolve_variable(&reference, definitions, resolved, visiting, depth + 1)
        })
    });

    visiting.remove(name);
    if let Some(color) = color.as_ref() {
        resolved.insert(name.to_owned(), color.clone());
    }
    color
}

fn parse_exact_reference(
    value: &str,
    definitions: &HashMap<String, VariableDefinition>,
) -> Option<String> {
    if let Some(inner) = value
        .strip_prefix("var(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let name = inner.split(',').next()?.trim();
        return definitions.contains_key(name).then(|| name.to_owned());
    }

    definitions.contains_key(value).then(|| value.to_owned())
}

fn scan_variable_references(
    text: &str,
    definitions: &HashMap<String, VariableDefinition>,
    context: ParseContext,
) -> Vec<(Span, String)> {
    let bytes = text.as_bytes();
    let mut references = Vec::new();
    let mut offset = 0;

    while offset < bytes.len() {
        if text[offset..].starts_with("var(") && token_boundary_before(bytes, offset) {
            if let Some(close) = matching_parenthesis(text, offset + 3) {
                let inner = &text[offset + 4..close];
                if let Some(name) = inner.split(',').next().map(str::trim) {
                    if definitions.contains_key(name) {
                        references.push((
                            Span {
                                start: offset,
                                end: close + 1,
                            },
                            name.to_owned(),
                        ));
                    }
                }
                offset = close + 1;
                continue;
            }
        }

        let prefixed = matches!(bytes[offset], b'$' | b'@')
            || (bytes[offset] == b'-' && bytes.get(offset + 1) == Some(&b'-'));
        if prefixed && token_boundary_before(bytes, offset) {
            let prefix_len = if bytes[offset] == b'-' { 2 } else { 1 };
            let end = take_while(bytes, offset + prefix_len, is_variable_character);
            let name = &text[offset..end];
            if definitions.contains_key(name) {
                references.push((Span { start: offset, end }, name.to_owned()));
            }
            offset = end;
            continue;
        }

        if (bytes[offset].is_ascii_alphabetic() || bytes[offset] == b'_')
            && token_boundary_before(bytes, offset)
        {
            let end = take_while(bytes, offset + 1, is_variable_character);
            let name = &text[offset..end];
            if definitions.contains_key(name) && named_color_context(text, offset, end, context) {
                references.push((Span { start: offset, end }, name.to_owned()));
            }
            offset = end;
            continue;
        }

        offset += text[offset..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(1);
    }

    references
}

fn take_while(bytes: &[u8], mut offset: usize, predicate: fn(&u8) -> bool) -> usize {
    while bytes.get(offset).is_some_and(predicate) {
        offset += 1;
    }
    offset
}

fn skip_ascii_whitespace(bytes: &[u8], mut offset: usize) -> usize {
    while bytes
        .get(offset)
        .is_some_and(|byte| byte.is_ascii_whitespace() && *byte != b'\n' && *byte != b'\r')
    {
        offset += 1;
    }
    offset
}

fn is_variable_character(byte: &u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn is_token_character(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'$' | b'@')
}

fn token_boundary_before(bytes: &[u8], offset: usize) -> bool {
    offset == 0 || !is_token_character(bytes[offset - 1])
}

fn token_boundary_after(bytes: &[u8], offset: usize) -> bool {
    bytes
        .get(offset)
        .is_none_or(|byte| !is_token_character(*byte))
}

fn previous_non_whitespace(bytes: &[u8], offset: usize) -> Option<(usize, u8)> {
    (0..offset)
        .rev()
        .find_map(|index| (!bytes[index].is_ascii_whitespace()).then_some((index, bytes[index])))
}

fn next_non_whitespace(bytes: &[u8], offset: usize) -> Option<(usize, u8)> {
    (offset..bytes.len())
        .find_map(|index| (!bytes[index].is_ascii_whitespace()).then_some((index, bytes[index])))
}

fn range_contains(outer: Range, inner: Range) -> bool {
    outer.start <= inner.start && outer.end >= inner.end
}

fn has_color_assignment_context(
    text: &str,
    declaration_start: usize,
    color_start: usize,
    color_end: usize,
    case_insensitive_property: bool,
) -> bool {
    if !has_assignment_before(
        text,
        declaration_start,
        color_start,
        case_insensitive_property,
    ) {
        return false;
    }

    let line_end = text[color_end..]
        .find(['\n', '\r'])
        .map_or(text.len(), |relative| color_end + relative);
    let tail = text[color_end..line_end].trim();
    !tail.ends_with(['.', '?', '!'])
}

fn appears_in_css_selector(text: &str, span: Span) -> bool {
    let statement_start = text[..span.start]
        .rfind([';', '{', '}', '\n', '\r'])
        .map_or(0, |offset| offset + 1);
    if has_assignment_before(text, statement_start, span.start, true) {
        return false;
    }

    let mut brackets: usize = 0;
    let mut quote = None;
    let mut escaped = false;

    for character in text[span.end..].chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
            continue;
        }
        match character {
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            '{' => return true,
            '=' if brackets == 0 => return false,
            ';' | '}' => return false,
            _ => {}
        }
    }
    false
}

fn has_assignment_before(
    text: &str,
    statement_start: usize,
    value_start: usize,
    case_insensitive_property: bool,
) -> bool {
    let prefix = &text[statement_start..value_start];
    let mut brackets: usize = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut assignment = None;

    for (offset, character) in prefix.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
            continue;
        }
        match character {
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            ':' | '=' if brackets == 0 => assignment = Some((offset, character)),
            _ => {}
        }
    }

    let Some((separator, delimiter)) = assignment else {
        return false;
    };
    if delimiter == '=' {
        return true;
    }

    let property = prefix[..separator].trim();
    let property = property
        .strip_prefix(['\'', '"'])
        .and_then(|property| property.strip_suffix(['\'', '"']))
        .unwrap_or(property);
    property.bytes().next().is_some_and(|byte| {
        byte.is_ascii_lowercase()
            || (case_insensitive_property && byte.is_ascii_uppercase())
            || matches!(byte, b'_' | b'-')
    }) && property
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn excluded_hex_fragment(text: &str, span: Span, context: ParseContext) -> bool {
    let stylesheet = is_stylesheet_at(text, span.start, context);
    is_url_fragment(text, span.start)
        || (stylesheet
            && (appears_in_css_selector(text, span)
                || inside_css_attribute_selector(text, span.start)))
        || (context.markup && inside_markup_href(text, span.start))
}

fn is_stylesheet_at(text: &str, offset: usize, context: ParseContext) -> bool {
    context.stylesheet || (context.markup && inside_style_element(text, offset))
}

fn inside_style_element(text: &str, offset: usize) -> bool {
    let prefix = text[..offset].to_ascii_lowercase();
    let Some(open) = prefix.rfind("<style") else {
        return false;
    };
    if prefix.rfind("</style").is_some_and(|close| close > open) {
        return false;
    }
    prefix[open..].contains('>')
}

fn is_url_fragment(text: &str, hash_start: usize) -> bool {
    let prefix = &text[..hash_start];
    let lowercase = prefix.to_ascii_lowercase();
    let Some(function_start) = lowercase.rfind("url(") else {
        return false;
    };
    token_boundary_before(prefix.as_bytes(), function_start)
        && !prefix[function_start + 4..].contains(')')
}

fn inside_css_attribute_selector(text: &str, hash_start: usize) -> bool {
    let prefix = &text[..hash_start];
    let open = prefix.rfind('[');
    let close = prefix.rfind(']');
    open.is_some() && open > close
}

fn inside_markup_href(text: &str, hash_start: usize) -> bool {
    let prefix = &text[..hash_start];
    let Some(tag_start) = prefix.rfind('<') else {
        return false;
    };
    if prefix.rfind('>').is_some_and(|tag_end| tag_end > tag_start) {
        return false;
    }

    let tag = &prefix[tag_start + 1..];
    let bytes = tag.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        while bytes
            .get(offset)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            offset += 1;
        }
        let name_start = offset;
        while bytes
            .get(offset)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':'))
        {
            offset += 1;
        }
        if name_start == offset {
            offset += 1;
            continue;
        }
        let attribute = &tag[name_start..offset];
        while bytes
            .get(offset)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            offset += 1;
        }
        if bytes.get(offset) != Some(&b'=') {
            continue;
        }
        offset += 1;
        while bytes
            .get(offset)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            offset += 1;
        }

        let href =
            attribute.eq_ignore_ascii_case("href") || attribute.eq_ignore_ascii_case("xlink:href");
        if matches!(bytes.get(offset), Some(b'\'' | b'"')) {
            let quote = bytes[offset];
            offset += 1;
            if let Some(close) = bytes[offset..].iter().position(|byte| *byte == quote) {
                offset += close + 1;
            } else {
                return href;
            }
        } else {
            let value_end = bytes[offset..]
                .iter()
                .position(|byte| byte.is_ascii_whitespace() || *byte == b'>')
                .map_or(bytes.len(), |relative| offset + relative);
            if value_end == bytes.len() {
                return href;
            }
            offset = value_end;
        }
    }
    false
}

fn span_within(inner: Span, outer: Span) -> bool {
    outer.start <= inner.start && outer.end >= inner.end
}

fn css_custom_property_scope(text: &str, definition_start: usize) -> Option<Span> {
    let mut braces = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut block_comment = false;
    let mut characters = text[..definition_start].char_indices().peekable();

    while let Some((offset, character)) = characters.next() {
        if block_comment {
            if character == '*' && characters.peek().is_some_and(|(_, next)| *next == '/') {
                characters.next();
                block_comment = false;
            }
            continue;
        }
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            }
            continue;
        }
        if character == '/' && characters.peek().is_some_and(|(_, next)| *next == '*') {
            characters.next();
            block_comment = true;
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == '{' {
            braces.push(offset);
        } else if character == '}' {
            braces.pop();
        }
    }

    let open_brace = braces.last().copied()?;
    let selector_start = text[..open_brace]
        .rfind([';', '{', '}'])
        .map_or(0, |offset| offset + 1);
    let selector = &text[selector_start..open_brace];
    let selector = if selector.to_ascii_lowercase().contains("<style") {
        selector
            .rsplit_once('>')
            .map_or(selector, |(_, selector)| selector)
    } else {
        selector
    };
    if contains_root_pseudo_class(selector) {
        None
    } else {
        Some(Span {
            start: open_brace,
            end: matching_closing_brace(text, open_brace).unwrap_or(text.len()),
        })
    }
}

fn matching_closing_brace(text: &str, open_brace: usize) -> Option<usize> {
    let mut depth = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut block_comment = false;
    let mut characters = text[open_brace..].char_indices().peekable();

    while let Some((relative, character)) = characters.next() {
        if block_comment {
            if character == '*' && characters.peek().is_some_and(|(_, next)| *next == '/') {
                characters.next();
                block_comment = false;
            }
            continue;
        }
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            }
            continue;
        }
        if character == '/' && characters.peek().is_some_and(|(_, next)| *next == '*') {
            characters.next();
            block_comment = true;
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == '{' {
            depth += 1;
        } else if character == '}' {
            depth -= 1;
            if depth == 0 {
                return Some(open_brace + relative + 1);
            }
        }
    }
    None
}

fn contains_root_pseudo_class(selector: &str) -> bool {
    selector.split(',').any(|selector| {
        let selector = selector.trim();
        !selector.starts_with('@')
            && !selector
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || matches!(byte, b'>' | b'+' | b'~'))
            && selector.match_indices(":root").any(|(offset, root)| {
                let end = offset + root.len();
                selector.as_bytes().get(end).is_none_or(|byte| {
                    !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-')
                })
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matched(text: &str) -> Vec<String> {
        parse(text).into_iter().map(|node| node.matched).collect()
    }

    fn matched_for(language_id: &str, text: &str) -> Vec<String> {
        parse_document(text, language_id)
            .into_iter()
            .map(|node| node.matched)
            .collect()
    }

    #[test]
    fn parses_required_css_color_syntaxes() {
        let text = r#"
            a: #abc #abcd #aabbcc #aabbccdd;
            b: 0xabc 0Xabcd 0xaabbcc 0XAABBCCDD;
            c: rgb(255 0 0 / 50%) rgba(10%, 20%, 30%, .4);
            d: hsl(120deg 50% 25% / .8) hsla(20, 30%, 40%, .5);
            e: hwb(90 10% 20% / .3) lab(50% 40 30) lch(50% 20 30deg);
            f: oklab(50% .1 .1 / .8) oklch(50% .1 120 / .8);
        "#;
        assert_eq!(parse(text).len(), 17);
    }

    #[test]
    fn rejects_partial_hex_and_identifier_literals() {
        assert!(parse("x: #12; y: #12345; z: #123456789; a: foo#fff;").is_empty());
        assert!(parse("x: 0x12; y: 0x12345; z: 0x123456789; a: foo0xfff;").is_empty());
    }

    #[test]
    fn named_colors_require_a_value_context() {
        let text = "red fox\nWarning: red means stop.\n.red {}\nlet red = token;\n\"red\": 1\ncolor: red;\nvalue = rebeccapurple;\n\"blue\"";
        assert_eq!(matched(text), vec!["red", "rebeccapurple", "blue"]);
    }

    #[test]
    fn excludes_css_id_selectors() {
        assert_eq!(
            matched(
                ".foo #abcdef {}\n#abcdef[data-active=\"true\"] {}\n[data-active=true] #abcdef:hover {}\n#abcdef,\n.other {}\n#abcdef\n{}\na { color: #abcdef; }"
            ),
            vec!["#abcdef"]
        );
    }

    #[test]
    fn excludes_url_and_language_specific_fragment_references() {
        assert!(matched_for("css", "a { fill: url(#abcdef); }").is_empty());
        assert!(matched_for("css", "a { fill: url(icons.svg?x=#fada55); }").is_empty());
        assert!(matched_for("css", r##"a { fill: url("#fada55"); }"##).is_empty());
        assert!(matched_for("html", r##"<a href="#abcdef">link</a>"##).is_empty());
        assert!(matched_for("html", r##"<a href="page.html?x=#fada55">link</a>"##).is_empty());
        assert!(matched_for("css", r##"[href="#abcdef"] {}"##).is_empty());
        assert_eq!(
            matched_for(
                "html",
                r##"<style>#dead, [href="#abcdef"] { COLOR: red; }</style>"##
            ),
            vec!["red"]
        );

        assert_eq!(
            matched_for("json", r##"{"color": "#abcdef"}"##),
            vec!["#abcdef"]
        );
        assert_eq!(
            matched_for("css", r##"a { color: "#abcdef"; }"##),
            vec!["#abcdef"]
        );
        assert_eq!(
            matched_for("html", r##"<div style="color:#abcdef"></div>"##),
            vec!["#abcdef"]
        );
    }

    #[test]
    fn preserves_semicolonless_colors_before_nested_rules() {
        let stylus = ".a {\n  color: #123\n  &:hover {\n    color: #456\n  }\n}";
        assert_eq!(matched(stylus), vec!["#123", "#456"]);
    }

    #[test]
    fn preserves_named_colors_in_css_shorthands() {
        assert_eq!(
            matched(
                "a { box-shadow: red 0 0 5px; text-shadow: 1px 1px blue inset; outline: solid green 1px; }"
            ),
            vec!["red", "blue", "green"]
        );
    }

    #[test]
    fn named_color_context_uses_document_language() {
        assert!(matched_for("markdown", "note: red means danger").is_empty());
        assert_eq!(matched_for("css", "a { COLOR: red; }"), vec!["red"]);
    }

    #[test]
    fn resolves_css_sass_less_and_stylus_variables() {
        let text = r#"
            --brand: #123456;
            --alias: var(--brand);
            $sass: oklch(60% .1 40);
            @less: $sass;
            stylus = @less
            a { color: var(--alias); background: $sass; border-color: @less; fill: stylus; }
        "#;
        let nodes = parse(text);
        let references = nodes
            .iter()
            .filter(|node| {
                matches!(
                    node.matched.as_str(),
                    "var(--alias)" | "$sass" | "@less" | "stylus"
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(references.len(), 6);
        assert!(references
            .windows(2)
            .all(|pair| pair[0].color.to_rgba8() != [0, 0, 0, 0]
                && pair[1].color.to_rgba8() != [0, 0, 0, 0]));
    }

    #[test]
    fn stylus_variables_override_named_color_spelling() {
        let nodes = parse("red = #0000ff\nbody\n  color: red");
        let reference = nodes
            .iter()
            .find(|node| node.matched == "red" && node.range.start.line == 2)
            .unwrap();
        assert_eq!(reference.color.to_rgba8(), [0, 0, 255, 255]);
    }

    #[test]
    fn ignores_cycles_and_unresolved_variables() {
        let text = "--a: var(--b); --b: var(--a); --missing: var(--nope); x: var(--a);";
        assert!(parse(text).is_empty());
    }

    #[test]
    fn ignores_ambiguous_duplicate_variable_definitions() {
        let text = "--brand: red;\n--brand: blue;\na { color: var(--brand); }";
        assert_eq!(matched(text), vec!["red", "blue"]);
    }

    #[test]
    fn css_custom_properties_do_not_leak_across_scopes() {
        let scoped = ".scope { --x: blue; } .other { color: var(--x); }";
        assert_eq!(matched(scoped), vec!["blue"]);

        let root = ":root { --x: blue; } .other { color: var(--x); }";
        assert_eq!(matched(root), vec!["blue", "var(--x)"]);

        let descendants = ":root .scope { --x: blue; } .other { color: var(--x); }";
        assert_eq!(matched(descendants), vec!["blue"]);

        let at_rule = "@supports selector(:root) { --x: blue; } a { color: var(--x); }";
        assert_eq!(matched(at_rule), vec!["blue"]);

        let overridden =
            ":root { --x: blue; } .dark { --x: red; color: var(--x); } .light { color: var(--x); }";
        assert_eq!(matched(overridden), vec!["blue", "red", "var(--x)"]);

        let markup = r##"<div style="--x: #f00; color: var(--x)"></div>"##;
        assert_eq!(matched_for("html", markup), vec!["#f00"]);

        let embedded = r##"<style>:root { --x: #f00; } a { color: var(--x); }</style>"##;
        assert_eq!(matched_for("html", embedded), vec!["#f00", "var(--x)"]);
    }

    #[test]
    fn resolved_css_variables_suppress_inactive_fallback_colors() {
        let nodes = parse("--x: blue; a { color: var(--x, red); }");
        assert_eq!(
            matched("--x: blue; a { color: var(--x, red); }"),
            vec!["blue", "var(--x, red)"]
        );
        assert_eq!(nodes[1].color.to_rgba8(), [0, 0, 255, 255]);

        assert_eq!(matched("a { color: var(--missing, red); }"), vec!["red"]);
    }

    #[test]
    fn reports_utf16_positions() {
        let text = "😀 café: #abc;\n文: rgb(1 2 3);";
        let nodes = parse(text);
        assert_eq!(
            nodes[0].range,
            Range::new(Position::new(0, 9), Position::new(0, 13))
        );
        assert_eq!(
            nodes[1].range,
            Range::new(Position::new(1, 3), Position::new(1, 13))
        );
    }

    #[test]
    fn parses_multiline_functions_with_utf16_ranges() {
        let nodes = parse("😀: rgb(\n  255 0 0\n);");
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].matched, "rgb(\n  255 0 0\n)");
        assert_eq!(
            nodes[0].range,
            Range::new(Position::new(0, 4), Position::new(2, 1))
        );
    }

    #[test]
    fn clamps_wide_gamut_colors_to_lsp_channels() {
        for value in ["lab(100% 125 125)", "oklab(100% .4 .4)"] {
            let node = parse(&format!("color: {value};")).remove(0);
            let color = node.lsp_color();
            assert!((0.0..=1.0).contains(&color.red), "{value}: {color:?}");
            assert!((0.0..=1.0).contains(&color.green), "{value}: {color:?}");
            assert!((0.0..=1.0).contains(&color.blue), "{value}: {color:?}");
            assert!((0.0..=1.0).contains(&color.alpha), "{value}: {color:?}");
        }
    }

    #[test]
    fn parses_gpui_normalized_colors_without_stealing_css_syntax() {
        assert_eq!(
            try_parse_gpui_color("rgb(0., 1., 0.2)"),
            Ok(Color::new(0.0, 1.0, 0.2, 1.0))
        );
        assert_eq!(
            try_parse_gpui_color("hsla(0.5, 1., 0.5, 0.3)"),
            Ok(Color::from_hsla(180.0, 1.0, 0.5, 0.3))
        );
        assert!(try_parse_gpui_color("rgb(255, 0, 0)").is_err());
        assert!(try_parse_gpui_color("rgb(100% 0% 0%)").is_err());
        assert_eq!(
            try_parse_color("rgb(0, 0, 1)").unwrap().to_rgba8(),
            [0, 0, 1, 255]
        );
        assert_eq!(
            try_parse_color("rgb(1 1 1)").unwrap().to_rgba8(),
            [1, 1, 1, 255]
        );
    }
}
