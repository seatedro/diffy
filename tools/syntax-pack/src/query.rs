use std::ops::Range;

use tree_sitter as ts;

const DROPPED_PREDICATES: &[&str] = &[
    "has-ancestor?",
    "not-has-ancestor?",
    "has-parent?",
    "not-has-parent?",
    "contains?",
    "any-contains?",
    "kind-eq?",
    "not-kind-eq?",
    "is?",
    "is-not?",
];

const MAX_REMOVALS: usize = 200;

pub struct Adapted {
    pub text: String,
    pub removed: usize,
}

pub fn adapt(language: &ts::Language, text: &str) -> Result<Adapted, String> {
    let mut text = translate_matches(text);
    let mut removed = 0;
    for name in DROPPED_PREDICATES {
        while let Some(at) = find_predicate(&text, name) {
            let pattern = toplevel_span(&text, at).ok_or("unbalanced query")?;
            text.replace_range(pattern, "");
            removed += 1;
        }
    }
    loop {
        match ts::Query::new(language, &text) {
            Ok(_) => return Ok(Adapted { text, removed }),
            Err(error) if removed < MAX_REMOVALS => {
                let span = match error.kind {
                    ts::QueryErrorKind::Predicate => predicate_span(&text, error.offset)
                        .or_else(|| toplevel_span(&text, error.offset)),
                    _ => toplevel_span(&text, error.offset),
                }
                .ok_or_else(|| format!("cannot repair query: {error}"))?;
                text.replace_range(span, "");
                removed += 1;
            }
            Err(error) => return Err(format!("query: {error}")),
        }
    }
}

fn find_predicate(text: &str, name: &str) -> Option<usize> {
    let needle = format!("(#{name}");
    let mut from = 0;
    while let Some(found) = text[from..].find(&needle) {
        let at = from + found;
        let after = text[at + needle.len()..].chars().next();
        if !in_comment_or_string(text, at) && after.is_none_or(|c| c.is_whitespace() || c == ')') {
            return Some(at);
        }
        from = at + needle.len();
    }
    None
}

fn in_comment_or_string(text: &str, at: usize) -> bool {
    let mut in_string = false;
    let mut chars = text[..at].char_indices().peekable();
    let mut in_comment = false;
    while let Some((_, c)) = chars.next() {
        if in_comment {
            in_comment = c != '\n';
        } else if in_string {
            match c {
                '\\' => {
                    chars.next();
                }
                '"' => in_string = false,
                _ => {}
            }
        } else {
            match c {
                ';' => in_comment = true,
                '"' => in_string = true,
                _ => {}
            }
        }
    }
    in_string || in_comment
}

fn toplevel_forms(text: &str) -> Vec<Range<usize>> {
    let mut forms = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    let mut in_string = false;
    let mut in_comment = false;
    let mut chars = text.char_indices();
    while let Some((i, c)) = chars.next() {
        if in_comment {
            in_comment = c != '\n';
            continue;
        }
        if in_string {
            match c {
                '\\' => {
                    chars.next();
                }
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            ';' => in_comment = true,
            '"' => {
                in_string = true;
                if depth == 0 && start.is_none() {
                    start = Some(i);
                }
            }
            '(' | '[' => {
                if depth == 0 && start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0
                    && let Some(s) = start.take()
                {
                    let end = i + 1;
                    let suffix = text[end..]
                        .chars()
                        .take_while(|c| matches!(c, '*' | '+' | '?'))
                        .count();
                    forms.push(s..end + suffix);
                }
            }
            c if depth == 0 && !c.is_whitespace() && start.is_none() => start = Some(i),
            c if depth == 0 && c.is_whitespace() => {
                if let Some(s) = start.take() {
                    forms.push(s..i);
                }
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        forms.push(s..text.len());
    }
    forms
}

fn toplevel_span(text: &str, offset: usize) -> Option<Range<usize>> {
    let forms = toplevel_forms(text);
    let index = forms
        .iter()
        .position(|form| form.start <= offset && offset < form.end.max(form.start + 1))
        .or_else(|| forms.iter().position(|form| form.start >= offset))?;
    let start = forms[index].start;
    let mut end = forms[index].end;
    for form in &forms[index + 1..] {
        if text[form.clone()].starts_with('@') {
            end = form.end;
        } else {
            break;
        }
    }
    Some(start..end)
}

fn predicate_span(text: &str, offset: usize) -> Option<Range<usize>> {
    let mut start = text[..=offset.min(text.len().saturating_sub(1))].rfind("(#")?;
    if let Some(next) = text[offset..].find("(#") {
        let next = offset + next;
        if close_of(text, start).is_none_or(|end| end <= offset) {
            start = next;
        }
    }
    let end = close_of(text, start)?;
    Some(start..end)
}

fn close_of(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut chars = text[open..].char_indices();
    while let Some((i, c)) = chars.next() {
        if in_string {
            match c {
                '\\' => {
                    chars.next();
                }
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn translate_matches(text: &str) -> String {
    let mut out = text.to_owned();
    for (from, to) in [
        ("not-lua-match?", "not-match?"),
        ("lua-match?", "match?"),
        ("not-vim-match?", "not-match?"),
        ("vim-match?", "match?"),
    ] {
        let lua = from.contains("lua");
        while let Some(at) = find_predicate(&out, from) {
            let Some(end) = close_of(&out, at) else { break };
            let body = &out[at + 2 + from.len()..end - 1];
            let replacement = body
                .find('"')
                .zip(body.rfind('"'))
                .filter(|(open, close)| open < close)
                .and_then(|(open, close)| {
                    let pattern = unescape(&body[open + 1..close]);
                    let regex = if lua {
                        lua_to_regex(&pattern)?
                    } else {
                        vim_to_regex(&pattern)
                    };
                    Some(format!(
                        "(#{to}{}\"{}\"{})",
                        &body[..open],
                        escape(&regex),
                        &body[close + 1..]
                    ))
                });
            match replacement {
                Some(replacement) => out.replace_range(at..end, &replacement),
                None => {
                    let pattern = toplevel_span(&out, at).unwrap_or(at..end);
                    out.replace_range(pattern, "");
                }
            }
        }
    }
    out
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn lua_class(c: char, in_set: bool) -> Option<String> {
    let body = match c.to_ascii_lowercase() {
        'a' => "A-Za-z",
        'd' => "0-9",
        'l' => "a-z",
        'u' => "A-Z",
        's' => " \\t\\n\\r\\f\\v",
        'w' => "A-Za-z0-9",
        'x' => "0-9A-Fa-f",
        'p' => "[:punct:]",
        'c' => "[:cntrl:]",
        _ => return None,
    };
    let negated = c.is_ascii_uppercase();
    Some(match (in_set, negated) {
        (true, false) => body.to_owned(),
        (true, true) => return None,
        (false, false) => format!("[{body}]"),
        (false, true) => format!("[^{body}]"),
    })
}

fn lua_to_regex(pattern: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    let mut in_set = false;
    while let Some(c) = chars.next() {
        match c {
            '%' => {
                let next = chars.next()?;
                if let Some(class) = lua_class(next, in_set) {
                    out.push_str(&class);
                } else if next.is_ascii_alphanumeric() {
                    return None;
                } else {
                    out.push('\\');
                    out.push(next);
                }
            }
            '[' if !in_set => {
                in_set = true;
                out.push('[');
                if chars.peek() == Some(&'^') {
                    out.push(chars.next()?);
                }
                if chars.peek() == Some(&']') {
                    chars.next();
                    out.push_str("\\]");
                }
            }
            ']' if in_set => {
                in_set = false;
                out.push(']');
            }
            '-' if !in_set => out.push_str("*?"),
            '\\' | '{' | '}' | '|' => {
                out.push('\\');
                out.push(c);
            }
            '[' | '&' | '~' if in_set => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    (!in_set).then_some(out)
}

fn vim_to_regex(pattern: &str) -> String {
    pattern
        .trim_start_matches("\\v")
        .replace("\\<", "\\b")
        .replace("\\>", "\\b")
        .replace("\\%(", "(?:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_patterns_translate_to_regex() {
        assert_eq!(
            lua_to_regex("^[A-Z][A-Z_0-9]*$").as_deref(),
            Some("^[A-Z][A-Z_0-9]*$")
        );
        assert_eq!(
            lua_to_regex("^%u%w*$").as_deref(),
            Some("^[A-Z][A-Za-z0-9]*$")
        );
        assert_eq!(
            lua_to_regex("^[%a_][%w_]*$").as_deref(),
            Some("^[A-Za-z_][A-Za-z0-9_]*$")
        );
        assert_eq!(lua_to_regex("^%.%-"), Some("^\\.\\-".to_owned()));
        assert_eq!(lua_to_regex("a.-b").as_deref(), Some("a.*?b"));
        assert_eq!(lua_to_regex("%b()"), None);
    }

    #[test]
    fn lua_match_becomes_match_with_an_escaped_regex() {
        let query = "((identifier) @constant\n  (#lua-match? @constant \"^%u[%u%d_]*$\"))\n";
        assert_eq!(
            translate_matches(query),
            "((identifier) @constant\n  (#match? @constant \"^[A-Z][A-Z0-9_]*$\"))\n"
        );
    }

    #[test]
    fn a_pattern_with_an_unsupported_predicate_is_removed_whole() {
        let query = "(a) @x\n((b) @y (#has-ancestor? @y c))\n(d) @z\n";
        let mut text = query.to_owned();
        let at = find_predicate(&text, "has-ancestor?").unwrap();
        let span = toplevel_span(&text, at).unwrap();
        text.replace_range(span, "");
        assert_eq!(text, "(a) @x\n\n(d) @z\n");
    }
}
