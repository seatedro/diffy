use std::fmt::Write as _;

use serde_json::Value;

pub const MAX_RANK_FILES: usize = 300;
pub const MAX_RANK_BYTES: usize = 16_000;
pub const RANK_MAX_TOKENS: u32 = 4_096;

const RANK_PROMPT: &str = "You order the changed files of a code change so a reviewer reads the most important ones first.
Put first: core logic, central modules and entry points that other code depends on, public APIs, data models, and changes to application behavior. Prefer substantial edits over trivial ones.
Put last: tests, docs, examples, configuration churn, generated code, and mechanical edits such as renames or formatting.
Each file line is: index, status (A added, M modified, D deleted, R renamed, C copied), added and removed line counts (? when unknown), path.
Reply with only a JSON object such as {\"ranked\":[2,0,1]} that lists every index exactly once, most important first.

Files:
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankCandidate {
    pub path: String,
    pub old_path: Option<String>,
    pub status: char,
    pub stats: Option<(u32, u32)>,
}

pub fn build_rank_message(candidates: &[RankCandidate]) -> (String, usize) {
    let mut message = String::from(RANK_PROMPT);
    let mut line = String::new();
    let mut sent = 0;
    for (index, file) in candidates.iter().take(MAX_RANK_FILES).enumerate() {
        line.clear();
        let _ = write!(line, "{index} {} ", file.status);
        match file.stats {
            Some((added, removed)) => {
                let _ = write!(line, "+{added} -{removed} ");
            }
            None => line.push_str("? "),
        }
        if let Some(old) = &file.old_path {
            line.push_str(old);
            line.push_str(" -> ");
        }
        line.push_str(&file.path);
        line.push('\n');
        if message.len() + line.len() > MAX_RANK_BYTES {
            break;
        }
        message.push_str(&line);
        sent += 1;
    }
    (message, sent)
}

pub fn parse_ranking(text: &str, count: usize) -> Result<Vec<u32>, String> {
    let value = extract_json(text).ok_or_else(|| "The model reply was not JSON.".to_owned())?;
    let items = match &value {
        Value::Array(items) => items,
        Value::Object(map) => map
            .get("ranked")
            .and_then(Value::as_array)
            .ok_or_else(|| "The model reply had no ranked list.".to_owned())?,
        _ => return Err("The model reply had no ranked list.".to_owned()),
    };
    let mut seen = vec![false; count];
    let mut ranked = Vec::new();
    for item in items {
        let index = match item {
            Value::Number(number) => number.as_u64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        };
        let Some(index) = index.and_then(|i| usize::try_from(i).ok()) else {
            continue;
        };
        if let Some(slot) = seen.get_mut(index)
            && !*slot
        {
            *slot = true;
            ranked.push(index as u32);
        }
    }
    if ranked.is_empty() {
        return Err("The model reply listed no known files.".to_owned());
    }
    Ok(ranked)
}

fn extract_json(text: &str) -> Option<Value> {
    let text = text.trim();
    if let Ok(value) = serde_json::from_str(text) {
        return Some(value);
    }
    [('{', '}'), ('[', ']')]
        .into_iter()
        .find_map(|(open, close)| {
            let start = text.find(open)?;
            let end = text.rfind(close)?;
            (start < end)
                .then(|| serde_json::from_str(&text[start..=end]).ok())
                .flatten()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(path: &str, stats: Option<(u32, u32)>) -> RankCandidate {
        RankCandidate {
            path: path.to_owned(),
            old_path: None,
            status: 'M',
            stats,
        }
    }

    #[test]
    fn message_lists_indexed_files_with_counts() {
        let files = [
            candidate("src/a.rs", Some((3, 1))),
            RankCandidate {
                old_path: Some("src/old.rs".to_owned()),
                status: 'R',
                ..candidate("src/new.rs", None)
            },
        ];
        let (message, sent) = build_rank_message(&files);
        assert_eq!(sent, 2);
        assert!(message.ends_with("0 M +3 -1 src/a.rs\n1 R ? src/old.rs -> src/new.rs\n"));
    }

    #[test]
    fn message_respects_file_and_byte_caps() {
        let many: Vec<RankCandidate> = (0..MAX_RANK_FILES + 50)
            .map(|i| candidate(&format!("src/f{i}.rs"), Some((1, 1))))
            .collect();
        let (message, sent) = build_rank_message(&many);
        assert_eq!(sent, MAX_RANK_FILES);
        assert!(message.len() <= MAX_RANK_BYTES);

        let long = "d/".repeat(200);
        let wide: Vec<RankCandidate> = (0..200)
            .map(|i| candidate(&format!("{long}f{i}.rs"), None))
            .collect();
        let (message, sent) = build_rank_message(&wide);
        assert!(sent > 0 && sent < wide.len());
        assert!(message.len() <= MAX_RANK_BYTES);
    }

    #[test]
    fn parses_ranked_object_and_bare_arrays() {
        assert_eq!(parse_ranking(r#"{"ranked":[2,0,1]}"#, 3), Ok(vec![2, 0, 1]));
        assert_eq!(parse_ranking("[1, 0]", 3), Ok(vec![1, 0]));
        assert_eq!(
            parse_ranking("```json\n{\"ranked\": [1, \"2\"]}\n```", 3),
            Ok(vec![1, 2])
        );
        assert_eq!(
            parse_ranking("Here you go: {\"ranked\":[0]} done", 1),
            Ok(vec![0])
        );
    }

    #[test]
    fn drops_invented_duplicate_and_malformed_indices() {
        assert_eq!(
            parse_ranking(r#"{"ranked":[7,1,1,-2,"x",0.5,0]}"#, 3),
            Ok(vec![1, 0])
        );
        assert!(parse_ranking(r#"{"ranked":[9]}"#, 3).is_err());
        assert!(parse_ranking(r#"{"order":[0]}"#, 3).is_err());
        assert!(parse_ranking("no json here", 3).is_err());
    }
}
