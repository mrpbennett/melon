use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::spec::CompletionCandidate;

/// Fuzzy matcher wrapping nucleo-matcher.
/// Filters and ranks candidates by match quality.
pub struct FuzzyMatcher {
    matcher: Matcher,
}

impl FuzzyMatcher {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    /// Filter and rank candidates against the given pattern.
    /// Returns candidates sorted by match score (best first).
    pub fn filter(&mut self, pattern: &str, candidates: Vec<CompletionCandidate>) -> Vec<ScoredCandidate> {
        if pattern.is_empty() {
            // No filtering needed — return all with equal score
            return candidates
                .into_iter()
                .map(|c| ScoredCandidate { candidate: c, score: 0 })
                .collect();
        }

        let pat = Pattern::new(pattern, CaseMatching::Smart, Normalization::Smart, AtomKind::Fuzzy);

        let mut scored: Vec<ScoredCandidate> = candidates
            .into_iter()
            .filter_map(|candidate| {
                let mut buf = Vec::new();
                let haystack = Utf32Str::new(&candidate.name, &mut buf);
                let score = pat.score(haystack, &mut self.matcher)?;
                Some(ScoredCandidate {
                    candidate,
                    score: score as i64,
                })
            })
            .collect();

        // Sort by score descending, then alphabetically for ties
        scored.sort_by(|a, b| {
            b.score.cmp(&a.score).then_with(|| a.candidate.name.cmp(&b.candidate.name))
        });

        scored
    }
}

impl Default for FuzzyMatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub candidate: CompletionCandidate,
    pub score: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::spec::CandidateKind;

    fn make_candidate(name: &str) -> CompletionCandidate {
        CompletionCandidate {
            name: name.to_string(),
            description: None,
            kind: CandidateKind::Subcommand,
        }
    }

    #[test]
    fn test_exact_match_scores_highest() {
        let mut matcher = FuzzyMatcher::new();
        let candidates = vec![
            make_candidate("commit"),
            make_candidate("compare"),
            make_candidate("component"),
        ];
        let results = matcher.filter("commit", candidates);
        assert!(!results.is_empty());
        assert_eq!(results[0].candidate.name, "commit");
    }

    #[test]
    fn test_prefix_match() {
        let mut matcher = FuzzyMatcher::new();
        let candidates = vec![
            make_candidate("commit"),
            make_candidate("compare"),
            make_candidate("clone"),
            make_candidate("checkout"),
        ];
        let results = matcher.filter("com", candidates);
        let names: Vec<&str> = results.iter().map(|r| r.candidate.name.as_str()).collect();
        assert!(names.contains(&"commit"));
        assert!(names.contains(&"compare"));
        // clone and checkout shouldn't match "com"
        assert!(!names.contains(&"clone"));
    }

    #[test]
    fn test_fuzzy_match() {
        let mut matcher = FuzzyMatcher::new();
        let candidates = vec![
            make_candidate("checkout"),
            make_candidate("cherry-pick"),
            make_candidate("commit"),
        ];
        let results = matcher.filter("chk", candidates);
        let names: Vec<&str> = results.iter().map(|r| r.candidate.name.as_str()).collect();
        // "checkout" should match "chk" fuzzily
        assert!(names.contains(&"checkout"));
    }

    #[test]
    fn test_empty_pattern_returns_all() {
        let mut matcher = FuzzyMatcher::new();
        let candidates = vec![
            make_candidate("a"),
            make_candidate("b"),
            make_candidate("c"),
        ];
        let results = matcher.filter("", candidates);
        assert_eq!(results.len(), 3);
    }
}
