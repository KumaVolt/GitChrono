//! Small fuzzy matcher for the command palette: case-insensitive subsequence
//! match, scored to favour prefixes, word starts and consecutive runs.

/// Higher is better; `None` when `query` isn't a subsequence of `text`.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars.iter().flat_map(|c| c.to_lowercase()).collect();
    if lower.len() != chars.len() {
        // Rare multi-char lowercase forms: fall back to a plain contains check.
        let flat: String = query.iter().collect();
        return text.to_lowercase().contains(&flat).then_some(1);
    }
    let mut score = 0;
    let mut qi = 0;
    let mut prev: Option<usize> = None;
    for (i, &c) in lower.iter().enumerate() {
        if qi == query.len() {
            break;
        }
        if c != query[qi] {
            continue;
        }
        score += 10;
        let word_start = i == 0 || !chars[i - 1].is_alphanumeric() || (chars[i].is_uppercase() && chars[i - 1].is_lowercase());
        if word_start {
            score += 15;
        }
        if i == 0 {
            score += 20;
        }
        match prev {
            Some(p) if p + 1 == i => score += 18,
            Some(p) => score -= (i - p - 1).min(8) as i32,
            None => score -= i.min(10) as i32,
        }
        prev = Some(i);
        qi += 1;
    }
    (qi == query.len()).then(|| score - (chars.len() as i32 / 8))
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn matches_subsequences_case_insensitively() {
        assert!(score("fch", "Fetch").is_some());
        assert!(score("FETCH", "fetch all").is_some());
        assert!(score("xyz", "Fetch").is_none());
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn prefers_prefixes_and_word_starts() {
        assert!(score("pu", "Push").unwrap() > score("pu", "Stash pop (Undo)").unwrap_or(i32::MIN));
        assert!(score("cb", "Create branch").unwrap() > score("cb", "Fetch both").unwrap());
        assert!(score("main", "Checkout main").unwrap() > score("main", "Checkout my-a-i-n").unwrap_or(i32::MIN));
    }
}
