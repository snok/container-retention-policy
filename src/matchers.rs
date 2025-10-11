use tracing::trace;
use wildmatch::WildMatchPattern;

/// Container for negative and positive "matcher"-expressions.
///
/// Matchers, in this context, are expressions like:
///
///    "foo"   -> select package "foo"
///    "foo*"  -> select packages starting with "foo"
///    "!foo"  -> select packages not called "foo"
///    "!foo*" -> select packages not starting with "foo"
///
/// i.e., glob-like patterns to include or exclude packages by.
///
/// We use matchers to select packages *and* to filter package versions.
/// When selecting packages, we filter by the package name, while for package versions,
/// we match by image tags.
///
/// Both our positive and negative matchers are vecs of [`WildMatchPattern`] from the [wildmatch]
/// crate.
///
/// When parsing matchers from strings, any string prefixed by `!` are considered
/// negative matchers, and anything else is considered positive.
#[derive(Debug, Clone)]
pub struct Matchers {
    pub positive: Vec<WildMatchPattern<'*', '?'>>,
    pub negative: Vec<WildMatchPattern<'*', '?'>>,
}

impl Matchers {
    /// Creates a new `Matchers` instance from a slice of filter strings.
    pub fn from(filters: &[String]) -> Self {
        trace!(filters=?filters, "Creating matchers from filters");
        Self {
            positive: filters
                .iter()
                .filter_map(|pattern| {
                    if pattern.starts_with('!') {
                        None
                    } else {
                        Some(WildMatchPattern::<'*', '?'>::new(pattern))
                    }
                })
                .collect(),
            negative: filters
                .iter()
                .filter_map(|pattern| pattern.strip_prefix('!').map(WildMatchPattern::<'*', '?'>::new))
                .collect(),
        }
    }

    /// Check whether there are any negative matches.
    pub fn negative_match(&self, value: &str) -> bool {
        self.negative.iter().any(|matcher| {
            if matcher.matches(value) {
                trace!("Negative filter `{matcher}` matched \"{value}\"");
                return true;
            };
            false
        })
    }

    /// Check whether there are any positive matches.
    pub fn positive_match(&self, value: &str) -> bool {
        self.positive.iter().any(|matcher| {
            if matcher.matches(value) {
                trace!("Positive filter `{matcher}` matched \"{value}\"");
                return true;
            }
            false
        })
    }

    pub fn is_empty(&self) -> bool {
        self.positive.is_empty() && self.negative.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_matchers() {
        // Exact filters should only match the exact string
        let matchers = Matchers::from(&vec![String::from("foo")]);

        assert!(!matchers.positive_match("fo"));
        assert!(matchers.positive_match("foo"));
        assert!(!matchers.positive_match("foos"));
        assert!(!matchers.positive_match("foosssss  $xas"));
        let matchers = Matchers::from(&vec![String::from("!foo")]);
        assert!(!matchers.negative_match("fo"));
        assert!(matchers.negative_match("foo"));
        assert!(!matchers.negative_match("foos"));
        assert!(!matchers.negative_match("foosssss  $xas"));

        // Wildcard filters should match the string without the wildcard, and with any postfix
        let matchers = Matchers::from(&vec![String::from("foo*")]);
        assert!(!matchers.positive_match("fo"));
        assert!(matchers.positive_match("foo"));
        assert!(matchers.positive_match("foos"));
        assert!(matchers.positive_match("foosssss  $xas"));
        let matchers = Matchers::from(&vec![String::from("!foo*")]);
        assert!(!matchers.negative_match("fo"));
        assert!(matchers.negative_match("foo"));
        assert!(matchers.negative_match("foos"));
        assert!(matchers.negative_match("foosssss  $xas"));

        // Filters with ? should match the string + one wildcard character
        let matchers = Matchers::from(&vec![String::from("foo?")]);
        assert!(!matchers.positive_match("fo"));
        assert!(!matchers.positive_match("foo"));
        assert!(matchers.positive_match("foos"));
        assert!(!matchers.positive_match("foosssss  $xas"));
        let matchers = Matchers::from(&vec![String::from("!foo?")]);
        assert!(!matchers.negative_match("fo"));
        assert!(!matchers.negative_match("foo"));
        assert!(matchers.negative_match("foos"));
        assert!(!matchers.negative_match("foosssss  $xas"));
    }
}
