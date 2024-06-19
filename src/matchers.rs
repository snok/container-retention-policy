use tracing::trace;
use wildmatch::WildMatchPattern;

/// Container for negative and positive "matcher"-expressions.
///
/// Matchers, in this context, are expressions like:
///
///     "foo"   -> select package "foo"
///     "foo*"  -> select packages starting with "foo"
///     "!foo"  -> select packages not called "foo"
///     "!foo*" -> select packages not starting with "foo"
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
#[derive(Debug)]
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
                .filter_map(|pattern| {
                    if let Some(without_prefix) = pattern.strip_prefix('!') {
                        Some(WildMatchPattern::<'*', '?'>::new(without_prefix))
                    } else {
                        None
                    }
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_create_filter_matchers() {
        // Exact filters should only match the exact string
        let matchers = Matchers::from(&vec![String::from("foo")]);
        assert!(!matchers.positive.iter().any(|m| m.matches("fo")));
        assert!(matchers.positive.iter().any(|m| m.matches("foo")));
        assert!(!matchers.positive.iter().any(|m| m.matches("foos")));
        assert!(!matchers.positive.iter().any(|m| m.matches("foosssss  $xas")));
        let matchers = Matchers::from(&vec![String::from("!foo")]);
        assert!(!matchers.negative.iter().any(|m| m.matches("fo")));
        assert!(matchers.negative.iter().any(|m| m.matches("foo")));
        assert!(!matchers.negative.iter().any(|m| m.matches("foos")));
        assert!(!matchers.negative.iter().any(|m| m.matches("foosssss  $xas")));

        // Wildcard filters should match the string without the wildcard, and with any postfix
        let matchers = Matchers::from(&vec![String::from("foo*")]);
        assert!(!matchers.positive.iter().any(|m| m.matches("fo")));
        assert!(matchers.positive.iter().any(|m| m.matches("foo")));
        assert!(matchers.positive.iter().any(|m| m.matches("foos")));
        assert!(matchers.positive.iter().any(|m| m.matches("foosssss  $xas")));
        let matchers = Matchers::from(&vec![String::from("!foo*")]);
        assert!(!matchers.negative.iter().any(|m| m.matches("fo")));
        assert!(matchers.negative.iter().any(|m| m.matches("foo")));
        assert!(matchers.negative.iter().any(|m| m.matches("foos")));
        assert!(matchers.negative.iter().any(|m| m.matches("foosssss  $xas")));

        // Filters with ? should match the string + one wildcard character
        let matchers = Matchers::from(&vec![String::from("foo?")]);
        assert!(!matchers.positive.iter().any(|m| m.matches("fo")));
        assert!(!matchers.positive.iter().any(|m| m.matches("foo")));
        assert!(matchers.positive.iter().any(|m| m.matches("foos")));
        assert!(!matchers.positive.iter().any(|m| m.matches("foosssss  $xas")));
        let matchers = Matchers::from(&vec![String::from("!foo?")]);
        assert!(!matchers.negative.iter().any(|m| m.matches("fo")));
        assert!(!matchers.negative.iter().any(|m| m.matches("foo")));
        assert!(matchers.negative.iter().any(|m| m.matches("foos")));
        assert!(!matchers.negative.iter().any(|m| m.matches("foosssss  $xas")));
    }
}
