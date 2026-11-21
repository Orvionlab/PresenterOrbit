//! The renderer's corpus-based spelling suggestions (cue-spelling-utils.ts). No dictionary dependency.
use std::ops::Range;
fn letter(c: char) -> bool {
    c.is_alphabetic()
        || matches!(c,'\u{0300}'..='\u{036f}'|'\u{1ab0}'..='\u{1aff}'|'\u{1dc0}'..='\u{1dff}'|'\u{20d0}'..='\u{20ff}'|'\u{fe20}'..='\u{fe2f}')
}
pub fn words(text: &str) -> Vec<(Range<usize>, String)> {
    let mut result = Vec::new();
    let mut start = None;
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if start.is_none() && letter(c) {
            start = Some(i);
        } else if !letter(c) && c != '\'' && c != '-' {
            if let Some(a) = start.take() {
                result.push((a..i, text[a..i].to_string()));
            }
        }
    }
    result
}
pub fn word_range(text: &str, selection: Range<usize>) -> Option<(Range<usize>, String)> {
    words(text)
        .into_iter()
        .find(|(r, _)| r.start <= selection.start && selection.end <= r.end)
}
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, c) in a.chars().enumerate() {
        let mut row = vec![i + 1; b.len() + 1];
        for (j, d) in b.iter().enumerate() {
            row[j + 1] = (prev[j + 1] + 1)
                .min(row[j] + 1)
                .min(prev[j] + usize::from(c != *d));
        }
        prev = row;
    }
    prev[b.len()]
}
pub fn suggestions(corpus: &[String], word: &str) -> Vec<String> {
    let target = word.to_lowercase();
    let mut seen = std::collections::HashSet::new();
    let mut ranked = Vec::new();
    for candidate in corpus {
        let normalized = candidate.to_lowercase();
        if normalized == target || !seen.insert(normalized.clone()) {
            continue;
        }
        let score = distance(&target, &normalized);
        if score <= 2.max(target.chars().count() / 3) {
            ranked.push((
                score,
                target.chars().count().abs_diff(normalized.chars().count()),
                candidate.clone(),
            ));
        }
    }
    ranked.sort();
    ranked.into_iter().take(10).map(|(_, _, s)| s).collect()
}
