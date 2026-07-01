//! Tag-based task selection (`--tags` / `--skip-tags`).
//!
//! [`TagFilter`] implements Ansible's tag-selection semantics:
//! - No `--tags` and no `--skip-tags` ⇒ run everything.
//! - `--tags foo` ⇒ run only tasks tagged `foo` or `always`; skip untagged.
//! - `--skip-tags bar` ⇒ skip tasks tagged `bar`; run everything else.
//! - `--tags all` ⇒ same as no filter (run everything not skipped).
//! - `--tags foo --skip-tags bar` ⇒ run `foo`-tagged tasks unless also tagged `bar`.

use std::collections::HashSet;

/// Tag selection from `--tags` / `--skip-tags`.
#[derive(Debug, Clone, Default)]
pub struct TagFilter {
    /// Tags requested via `--tags` (empty ⇒ no positive filter).
    tags: HashSet<String>,
    /// Tags excluded via `--skip-tags`.
    skip_tags: HashSet<String>,
}

impl TagFilter {
    /// Build a filter from comma-separated `--tags` / `--skip-tags` values.
    #[must_use]
    pub fn from_cli(tags: Option<&str>, skip_tags: Option<&str>) -> Self {
        Self {
            tags: parse_csv(tags),
            skip_tags: parse_csv(skip_tags),
        }
    }

    /// An empty filter (run everything) — the default when neither flag is set.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Whether both selectors are empty (no filtering at all).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.skip_tags.is_empty()
    }

    /// Whether a task whose effective tags are `tags` should run.
    ///
    /// Effective tags = the task's own tags ∪ enclosing block tags (the caller
    /// combines them before calling).
    #[must_use]
    pub fn should_run(&self, tags: &[&str]) -> bool {
        // `--skip-tags` always takes precedence.
        if tags.iter().any(|t| self.skip_tags.contains(*t)) {
            return false;
        }
        // No positive `--tags` filter ⇒ everything not skipped runs.
        if self.tags.is_empty() {
            return true;
        }
        // `--tags all` ⇒ run everything not skipped.
        if self.tags.contains("all") {
            return true;
        }
        // With a specific `--tags` list: run if the task has any requested tag
        // or carries the special `always` tag.
        tags.iter().any(|t| self.tags.contains(*t)) || tags.contains(&"always")
    }
}

/// Parse a comma-separated CLI string into a set of tag names.
fn parse_csv(value: Option<&str>) -> HashSet<String> {
    let Some(s) = value else {
        return HashSet::new();
    };
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_filter_runs_everything() {
        let f = TagFilter::none();
        assert!(f.should_run(&[]));
        assert!(f.should_run(&["foo"]));
        assert!(f.is_empty());
    }

    #[test]
    fn tags_filter_skips_untagged() {
        let f = TagFilter::from_cli(Some("web"), None);
        assert!(!f.should_run(&[])); // untagged task skipped
        assert!(f.should_run(&["web"]));
        assert!(!f.should_run(&["db"]));
    }

    #[test]
    fn tags_all_runs_everything() {
        let f = TagFilter::from_cli(Some("all"), None);
        assert!(f.should_run(&[]));
        assert!(f.should_run(&["anything"]));
    }

    #[test]
    fn always_tag_always_runs() {
        let f = TagFilter::from_cli(Some("web"), None);
        assert!(f.should_run(&["always"]));
        assert!(f.should_run(&["web", "always"]));
    }

    #[test]
    fn skip_tags_excludes() {
        let f = TagFilter::from_cli(None, Some("slow"));
        assert!(f.should_run(&[]));
        assert!(f.should_run(&["web"]));
        assert!(!f.should_run(&["slow"]));
        assert!(!f.should_run(&["web", "slow"]));
    }

    #[test]
    fn tags_and_skip_tags_combine() {
        let f = TagFilter::from_cli(Some("web"), Some("slow"));
        assert!(f.should_run(&["web"]));
        assert!(!f.should_run(&["web", "slow"]));
        assert!(!f.should_run(&["db"]));
    }

    #[test]
    fn skip_always_blocks_always_tag() {
        let f = TagFilter::from_cli(Some("web"), Some("always"));
        assert!(f.should_run(&["web"]));
        assert!(!f.should_run(&["always"]));
    }

    #[test]
    fn from_cli_parses_comma_separated() {
        let f = TagFilter::from_cli(Some("a, b ,c"), Some(" x , y"));
        assert!(f.should_run(&["a"]));
        assert!(f.should_run(&["b"]));
        assert!(f.should_run(&["c"]));
        assert!(!f.should_run(&["x"]));
    }
}
