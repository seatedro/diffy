pub fn fuzzy_score(pattern: &str, candidate: &str) -> Option<i32> {
    if pattern.is_empty() {
        return Some(0);
    }
    if candidate.is_empty() {
        return None;
    }

    const SEQUENTIAL_BONUS: i32 = 15;
    const SEPARATOR_BONUS: i32 = 30;
    const CAMEL_BONUS: i32 = 30;
    const FIRST_LETTER_BONUS: i32 = 15;
    const LEADING_LETTER_PENALTY: i32 = -5;
    const MAX_LEADING_LETTER_PENALTY: i32 = -15;
    const UNMATCHED_LETTER_PENALTY: i32 = -1;

    let pattern: Vec<char> = pattern.chars().collect();
    let candidate: Vec<char> = candidate.chars().collect();
    let mut score = 0;
    let mut pattern_index = 0;
    let mut prev_matched = false;
    let mut prev_separator = true;

    for (candidate_index, &candidate_char) in candidate.iter().enumerate() {
        let current_separator = is_separator(candidate_char);
        if pattern_index < pattern.len()
            && pattern[pattern_index].eq_ignore_ascii_case(&candidate_char)
        {
            let mut letter_score = 0;
            if candidate_index == 0 {
                letter_score += FIRST_LETTER_BONUS;
            }
            if prev_matched {
                letter_score += SEQUENTIAL_BONUS;
            }
            if prev_separator {
                letter_score += SEPARATOR_BONUS;
            }
            if pattern_index > 0 && is_camel_boundary(&candidate, candidate_index) {
                letter_score += CAMEL_BONUS;
            }
            score += letter_score;
            pattern_index += 1;
            prev_matched = true;
        } else {
            score += UNMATCHED_LETTER_PENALTY;
            prev_matched = false;
        }
        prev_separator = current_separator;
    }

    if pattern_index != pattern.len() {
        return None;
    }

    let first_match_index = candidate
        .iter()
        .position(|candidate_char| pattern[0].eq_ignore_ascii_case(candidate_char))
        .unwrap_or(0) as i32;
    score += (LEADING_LETTER_PENALTY * first_match_index).max(MAX_LEADING_LETTER_PENALTY);
    Some(score)
}

fn is_separator(ch: char) -> bool {
    matches!(ch, '/' | '\\' | '_' | '-' | '.' | ' ')
}

fn is_camel_boundary(candidate: &[char], index: usize) -> bool {
    index > 0 && candidate[index].is_ascii_uppercase() && !candidate[index - 1].is_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::fuzzy_score;

    #[test]
    fn exact_match_scores_higher_than_middle_match() {
        assert!(fuzzy_score("main", "main").unwrap() > fuzzy_score("main", "omain").unwrap());
    }

    #[test]
    fn prefix_match_beats_middle_match() {
        assert!(
            fuzzy_score("src", "src/app/Foo.cpp").unwrap()
                > fuzzy_score("src", "lib/src/Bar.cpp").unwrap()
        );
    }

    #[test]
    fn subsequence_matches() {
        assert!(fuzzy_score("cmk", "CMakeLists.txt").unwrap() > 0);
    }

    #[test]
    fn no_match_returns_none() {
        assert_eq!(fuzzy_score("xyz", "CMakeLists.txt"), None);
    }
}
