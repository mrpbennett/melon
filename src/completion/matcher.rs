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
    pub fn filter(
        &mut self,
        pattern: &str,
        candidates: Vec<CompletionCandidate>,
    ) -> Vec<ScoredCandidate> {
        let mut scored = Vec::with_capacity(candidates.len());
        if pattern.is_empty() {
            for candidate in candidates {
                scored.push(ScoredCandidate {
                    candidate,
                    score: 0,
                });
            }
        } else {
            let pat = Pattern::new(
                pattern,
                CaseMatching::Smart,
                Normalization::Smart,
                AtomKind::Fuzzy,
            );
            let mut utf32_buf = Vec::new();
            for candidate in candidates {
                utf32_buf.clear();
                let haystack = Utf32Str::new(&candidate.name, &mut utf32_buf);
                if let Some(score) = pat.score(haystack, &mut self.matcher) {
                    scored.push(ScoredCandidate {
                        candidate,
                        score: score as i64,
                    });
                }
            }
        }

        // Sort by score descending, then spec priority, then alphabetically.
        scored.sort_unstable_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| b.candidate.priority.cmp(&a.candidate.priority))
                .then_with(|| a.candidate.name.cmp(&b.candidate.name))
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
            insert_value: None,
            display_name: None,
            description: None,
            icon: None,
            priority: 50,
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

    #[test]
    fn test_priority_breaks_score_ties() {
        let mut matcher = FuzzyMatcher::new();
        let mut low = make_candidate("alpha");
        low.priority = 10;
        let mut high = make_candidate("beta");
        high.priority = 90;

        let results = matcher.filter("", vec![low, high]);
        assert_eq!(results[0].candidate.name, "beta");
    }
}
