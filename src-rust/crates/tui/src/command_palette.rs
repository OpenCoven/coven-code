//! Presentation-only command catalog for TUI command discovery surfaces.

use claurst_core::CommandContextSignal;
use std::collections::{HashMap, HashSet};

/// Owned command metadata consumed by the prompt, help overlay, and palette.
///
/// This DTO deliberately does not expose command-crate types so the TUI stays
/// independent from command registration and execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPaletteEntry {
    pub path: String,
    pub aliases: Vec<String>,
    pub summary: String,
    pub usage: String,
    pub category: String,
    pub keywords: Vec<String>,
    pub source: String,
    pub suggest_when: Vec<CommandContextSignal>,
    pub children: Vec<Self>,
}

impl CommandPaletteEntry {
    /// Return this entry followed by its descendants in catalog order.
    pub fn flattened(&self) -> Vec<&Self> {
        let mut entries = vec![self];
        for child in &self.children {
            entries.extend(child.flattened());
        }
        entries
    }
}

/// Flatten a presentation catalog, preserving parent-before-child order.
pub fn flatten_command_catalog(catalog: &[CommandPaletteEntry]) -> Vec<&CommandPaletteEntry> {
    catalog
        .iter()
        .flat_map(CommandPaletteEntry::flattened)
        .collect()
}

/// Session and environment signals used to rank command discovery results.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandPaletteContext {
    pub signals: HashSet<CommandContextSignal>,
    pub recent_paths: Vec<String>,
}

const CATEGORY_ORDER: &[&str] = &[
    "Start & Navigate",
    "Work & Review",
    "Conversation",
    "Account & Models",
    "Customize",
    "Extend",
    "Coven & Advanced",
];

/// Stable user-facing command category order.
pub const fn command_category_order() -> &'static [&'static str] {
    CATEGORY_ORDER
}

fn category_rank(category: &str) -> usize {
    CATEGORY_ORDER
        .iter()
        .position(|candidate| *candidate == category)
        .unwrap_or(CATEGORY_ORDER.len())
}

fn recency_scores(paths: &[String]) -> HashMap<&str, i32> {
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| (path.as_str(), (50 - (index as i32 * 5)).max(5)))
        .collect()
}

fn textual_score(entry: &CommandPaletteEntry, query: &str) -> Option<i32> {
    let path = entry.path.to_lowercase();
    if path == query {
        return Some(10_000);
    }
    if path.starts_with(query) {
        return Some(8_000);
    }

    let aliases = entry
        .aliases
        .iter()
        .map(|alias| alias.to_lowercase())
        .collect::<Vec<_>>();
    if aliases.iter().any(|alias| alias == query) {
        return Some(7_500);
    }
    if aliases.iter().any(|alias| alias.starts_with(query)) {
        return Some(7_000);
    }

    let query_words = query.split_whitespace().collect::<Vec<_>>();
    let searchable = std::iter::once(entry.path.as_str())
        .chain(entry.keywords.iter().map(String::as_str))
        .chain(std::iter::once(entry.summary.as_str()))
        .chain(std::iter::once(entry.category.as_str()))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let searchable_words = searchable
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<HashSet<_>>();
    if !query_words.is_empty()
        && query_words
            .iter()
            .all(|word| searchable_words.contains(word))
    {
        let path_words = path.split_whitespace().collect::<HashSet<_>>();
        let path_matches = query_words
            .iter()
            .filter(|word| path_words.contains(**word))
            .count() as i32;
        return Some(5_000 + path_matches * 100);
    }

    None
}

fn edit_distance(left: &str, right: &str) -> usize {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];

    for (left_index, left_byte) in left.iter().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_byte) in right.iter().enumerate() {
            current[right_index + 1] = if left_byte == right_byte {
                previous[right_index]
            } else {
                1 + previous[right_index]
                    .min(current[right_index])
                    .min(previous[right_index + 1])
            };
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right.len()]
}

fn typo_score(entry: &CommandPaletteEntry, query: &str) -> Option<i32> {
    let candidates = std::iter::once(entry.path.as_str())
        .chain(entry.path.split_whitespace())
        .chain(entry.aliases.iter().map(String::as_str));
    let distance = candidates
        .map(|candidate| edit_distance(query, &candidate.to_lowercase()))
        .min()?;
    let threshold = if query.len() <= 4 { 1 } else { 2 };
    (distance <= threshold).then_some(1_000 - distance as i32)
}

fn score_entry(
    entry: &CommandPaletteEntry,
    query: &str,
    context: &CommandPaletteContext,
    recency: &HashMap<&str, i32>,
    allow_typo: bool,
) -> Option<i32> {
    let mut score = if query.is_empty() {
        0
    } else if let Some(score) = textual_score(entry, query) {
        score
    } else if allow_typo {
        typo_score(entry, query)?
    } else {
        return None;
    };

    score += entry
        .suggest_when
        .iter()
        .filter(|signal| context.signals.contains(signal))
        .count() as i32
        * 500;
    score += recency.get(entry.path.as_str()).copied().unwrap_or(0);
    if query.is_empty() && entry.category == "Coven & Advanced" {
        score -= 1_000;
    }
    Some(score)
}

/// Rank flattened canonical command entries deterministically.
///
/// Canonical paths are deduplicated before the result is truncated. When no
/// direct intent match exists, a low-scoring edit-distance fallback keeps typo
/// suggestions such as `logn` useful without perturbing exact matches.
pub fn rank_commands<'a>(
    catalog: &'a [CommandPaletteEntry],
    query: &str,
    context: &CommandPaletteContext,
    limit: usize,
) -> Vec<&'a CommandPaletteEntry> {
    let query = query.trim().trim_start_matches('/').trim().to_lowercase();
    let recency = recency_scores(&context.recent_paths);
    let flattened = flatten_command_catalog(catalog);
    let allow_typo = !query.is_empty()
        && !flattened
            .iter()
            .any(|entry| textual_score(entry, &query).is_some());
    let mut seen = HashSet::new();
    let mut scored = flattened
        .into_iter()
        .filter(|entry| seen.insert(entry.path.as_str()))
        .filter_map(|entry| {
            score_entry(entry, &query, context, &recency, allow_typo).map(|score| (score, entry))
        })
        .collect::<Vec<_>>();

    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| category_rank(&left.category).cmp(&category_rank(&right.category)))
            .then_with(|| left.path.cmp(&right.path))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, entry)| entry)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use claurst_core::CommandContextSignal::{EmptySession, MissingAgentsFile, Unauthenticated};
    use std::collections::HashSet;

    fn entry(
        path: &str,
        aliases: &[&str],
        summary: &str,
        category: &str,
        keywords: &[&str],
        suggest_when: &[claurst_core::CommandContextSignal],
    ) -> CommandPaletteEntry {
        CommandPaletteEntry {
            path: path.to_string(),
            aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
            summary: summary.to_string(),
            usage: format!("/{path}"),
            category: category.to_string(),
            keywords: keywords
                .iter()
                .map(|keyword| (*keyword).to_string())
                .collect(),
            source: "built_in".to_string(),
            suggest_when: suggest_when.to_vec(),
            children: Vec::new(),
        }
    }

    #[test]
    fn flattening_preserves_parent_before_children() {
        let catalog = vec![CommandPaletteEntry {
            path: "review".to_string(),
            aliases: vec!["r".to_string()],
            summary: "Review changes".to_string(),
            usage: "/review".to_string(),
            category: "Work & Review".to_string(),
            keywords: vec!["review".to_string()],
            source: "built_in".to_string(),
            suggest_when: Vec::new(),
            children: vec![CommandPaletteEntry {
                path: "review security".to_string(),
                aliases: Vec::new(),
                summary: "Review security".to_string(),
                usage: "/review security".to_string(),
                category: "Work & Review".to_string(),
                keywords: vec!["security".to_string()],
                source: "built_in".to_string(),
                suggest_when: Vec::new(),
                children: Vec::new(),
            }],
        }];

        assert_eq!(
            flatten_command_catalog(&catalog)
                .into_iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["review", "review security"]
        );
    }

    #[test]
    fn empty_query_ranks_contextual_commands_and_caps_results() {
        let mut account = entry(
            "account",
            &[],
            "Manage accounts",
            "Account & Models",
            &["account"],
            &[],
        );
        account.children = vec![entry(
            "account connect",
            &["connect"],
            "Connect a provider account",
            "Account & Models",
            &["connect", "auth"],
            &[Unauthenticated, EmptySession, MissingAgentsFile],
        )];
        let mut session = entry(
            "session",
            &[],
            "Manage sessions",
            "Start & Navigate",
            &["session"],
            &[],
        );
        session.children = vec![entry(
            "session resume",
            &["resume"],
            "Resume a saved session",
            "Start & Navigate",
            &["resume"],
            &[EmptySession],
        )];
        let mut catalog = vec![
            account,
            session,
            entry(
                "init",
                &[],
                "Initialize project instructions",
                "Start & Navigate",
                &["agents", "project"],
                &[EmptySession, MissingAgentsFile],
            ),
        ];
        catalog.extend((0..10).map(|index| {
            entry(
                &format!("other-{index}"),
                &[],
                "Other command",
                "Extend",
                &["other"],
                &[],
            )
        }));
        let context = CommandPaletteContext {
            signals: HashSet::from([Unauthenticated, EmptySession, MissingAgentsFile]),
            recent_paths: Vec::new(),
        };

        let ranked = rank_commands(&catalog, "", &context, 8);
        let paths = ranked
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();

        assert!(ranked.len() <= 8);
        assert_eq!(paths[0], "account connect");
        assert!(paths.contains(&"session resume"));
        assert!(paths.contains(&"init"));
    }

    #[test]
    fn intent_queries_rank_expected_canonical_paths_first() {
        let catalog = vec![
            entry(
                "account login",
                &["login"],
                "Sign in to a provider",
                "Account & Models",
                &["sign in", "auth"],
                &[],
            ),
            entry(
                "config theme",
                &["theme"],
                "Select the display theme",
                "Customize",
                &["change theme", "appearance"],
                &[],
            ),
            entry(
                "rewind",
                &[],
                "Rewind recent changes",
                "Work & Review",
                &["go back", "undo"],
                &[],
            ),
        ];
        let context = CommandPaletteContext::default();

        for (query, expected) in [
            ("sign in", "account login"),
            ("change theme", "config theme"),
            ("go back", "rewind"),
        ] {
            assert_eq!(
                rank_commands(&catalog, query, &context, 8)[0].path,
                expected
            );
        }
    }

    #[test]
    fn aliases_match_without_creating_duplicate_rows() {
        let catalog = vec![
            entry(
                "account login",
                &["login", "sign-in"],
                "Sign in",
                "Account & Models",
                &["auth"],
                &[],
            ),
            entry(
                "learn",
                &["login guide"],
                "Learn Coven",
                "Extend",
                &["guide"],
                &[],
            ),
        ];

        let ranked = rank_commands(&catalog, "login", &CommandPaletteContext::default(), 8);
        let login_rows = ranked
            .iter()
            .filter(|entry| entry.path == "account login")
            .count();

        assert_eq!(ranked[0].path, "account login");
        assert_eq!(login_rows, 1);
    }

    #[test]
    fn textual_score_bands_have_exact_precedence() {
        let exact_path = entry(
            "login",
            &[],
            "Exact path",
            "Start & Navigate",
            &["exact"],
            &[],
        );
        let path_prefix = entry(
            "login-now",
            &[],
            "Path prefix",
            "Start & Navigate",
            &["prefix"],
            &[],
        );
        let exact_alias = entry(
            "account",
            &["login"],
            "Exact alias",
            "Account & Models",
            &["alias"],
            &[],
        );
        let alias_prefix = entry(
            "connect",
            &["login-now"],
            "Alias prefix",
            "Account & Models",
            &["alias"],
            &[],
        );
        let whole_word = entry("help", &[], "Whole word", "Extend", &["login"], &[]);
        let typo = entry("login", &[], "Typo", "Start & Navigate", &["typo"], &[]);

        let scores = [
            textual_score(&exact_path, "login"),
            textual_score(&path_prefix, "login"),
            textual_score(&exact_alias, "login"),
            textual_score(&alias_prefix, "login"),
            textual_score(&whole_word, "login"),
            typo_score(&typo, "logn"),
        ];

        assert_eq!(
            scores,
            [
                Some(10_000),
                Some(8_000),
                Some(7_500),
                Some(7_000),
                Some(5_000),
                Some(999),
            ]
        );
        assert!(scores.windows(2).all(|pair| pair[0] > pair[1]));
    }

    #[test]
    fn equal_scores_use_category_order_then_canonical_path() {
        let catalog = vec![
            entry("zeta", &[], "Zeta", "Work & Review", &["same"], &[]),
            entry("beta", &[], "Beta", "Start & Navigate", &["same"], &[]),
            entry("alpha", &[], "Alpha", "Start & Navigate", &["same"], &[]),
        ];

        assert_eq!(
            rank_commands(&catalog, "same", &CommandPaletteContext::default(), 8)
                .into_iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta", "zeta"]
        );
    }

    #[test]
    fn duplicate_canonical_paths_keep_only_the_first_catalog_entry() {
        let catalog = vec![
            entry(
                "review",
                &[],
                "First review",
                "Work & Review",
                &["review"],
                &[],
            ),
            entry(
                "review",
                &["duplicate"],
                "Duplicate review",
                "Extend",
                &["review"],
                &[],
            ),
        ];

        let ranked = rank_commands(&catalog, "review", &CommandPaletteContext::default(), 8);

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].summary, "First review");
    }
}
