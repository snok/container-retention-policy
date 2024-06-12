use tracing::trace;
use wildmatch::WildMatchPattern;

#[derive(Debug)]
pub struct Matchers {
    pub positive: Vec<WildMatchPattern<'*', '?'>>,
    pub negative: Vec<WildMatchPattern<'*', '?'>>,
}

pub fn create_filter_matchers(filters: &[String]) -> Matchers {
    trace!(
        filters=?filters,
        "Creating matchers from filters"
    );
    Matchers {
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_create_filter_matchers() {
        // Exact filters should only match the exact string
        let matchers = create_filter_matchers(&vec![String::from("foo")]);
        assert!(!matchers.positive.iter().any(|m| m.matches("fo")));
        assert!(matchers.positive.iter().any(|m| m.matches("foo")));
        assert!(!matchers.positive.iter().any(|m| m.matches("foos")));
        assert!(!matchers.positive.iter().any(|m| m.matches("foosssss  $xas")));
        let matchers = create_filter_matchers(&vec![String::from("!foo")]);
        assert!(!matchers.negative.iter().any(|m| m.matches("fo")));
        assert!(matchers.negative.iter().any(|m| m.matches("foo")));
        assert!(!matchers.negative.iter().any(|m| m.matches("foos")));
        assert!(!matchers.negative.iter().any(|m| m.matches("foosssss  $xas")));

        // Wildcard filters should match the string without the wildcard, and with any postfix
        let matchers = create_filter_matchers(&vec![String::from("foo*")]);
        assert!(!matchers.positive.iter().any(|m| m.matches("fo")));
        assert!(matchers.positive.iter().any(|m| m.matches("foo")));
        assert!(matchers.positive.iter().any(|m| m.matches("foos")));
        assert!(matchers.positive.iter().any(|m| m.matches("foosssss  $xas")));
        let matchers = create_filter_matchers(&vec![String::from("!foo*")]);
        assert!(!matchers.negative.iter().any(|m| m.matches("fo")));
        assert!(matchers.negative.iter().any(|m| m.matches("foo")));
        assert!(matchers.negative.iter().any(|m| m.matches("foos")));
        assert!(matchers.negative.iter().any(|m| m.matches("foosssss  $xas")));

        // Filters with ? should match the string + one wildcard character
        let matchers = create_filter_matchers(&vec![String::from("foo?")]);
        assert!(!matchers.positive.iter().any(|m| m.matches("fo")));
        assert!(!matchers.positive.iter().any(|m| m.matches("foo")));
        assert!(matchers.positive.iter().any(|m| m.matches("foos")));
        assert!(!matchers.positive.iter().any(|m| m.matches("foosssss  $xas")));
        let matchers = create_filter_matchers(&vec![String::from("!foo?")]);
        assert!(!matchers.negative.iter().any(|m| m.matches("fo")));
        assert!(!matchers.negative.iter().any(|m| m.matches("foo")));
        assert!(matchers.negative.iter().any(|m| m.matches("foos")));
        assert!(!matchers.negative.iter().any(|m| m.matches("foosssss  $xas")));
    }
}
