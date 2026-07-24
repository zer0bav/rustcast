//! Result scoring — the tiered matcher that decides what a query surfaces.
//!
//! A plain fuzzy score is a bad launcher ranking: typing `si` makes the matcher
//! happily accept "Exten**si**on Manager" and "Qt A**s**s**i**stant" at roughly
//! the same score as "**Si**gnal", so the app you actually want is buried. So
//! matches are bucketed into *tiers* by quality — exact, prefix, word-start,
//! acronym, substring, fuzzy, keyword-only — with gaps wide enough that a better
//! kind of match always outranks a worse one. Within a tier, usage history
//! ([`crate::frecency`], capped at [`BOOST_MAX`]) decides, so your most-used app
//! wins ties. Shorter names break the remaining ties.

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

pub fn matcher() -> SkimMatcherV2 {
    SkimMatcherV2::default()
}

/// Query equals the whole name ("signal" → Signal).
pub const EXACT: i64 = 10_000;
/// Name starts with the query ("si" → **Si**gnal).
pub const PREFIX: i64 = 8_000;
/// Some word of the name starts with the query ("code" → Visual Studio **Code**).
pub const WORD: i64 = 6_500;
/// The initials match ("vsc" → **V**isual **S**tudio **C**ode).
pub const ACRONYM: i64 = 5_500;
/// The name contains the query anywhere.
pub const SUBSTR: i64 = 4_500;
/// Fuzzy (subsequence) match on the name.
pub const FUZZY: i64 = 2_500;
/// Only the hidden keywords matched (GenericName, Keywords=, aliases…).
pub const KEYWORD: i64 = 1_200;
/// Baseline for a browse-everything (empty query) listing, so frecency alone
/// orders the root.
pub const IDLE: i64 = 400;
/// Ceiling on the usage boost. Must stay below the smallest tier gap (1_000) so
/// history reorders *within* a tier but never promotes a worse match kind.
pub const BOOST_MAX: i64 = 900;

/// Score `query` against an item's display `name` plus hidden `keywords`.
/// `None` means "no match — drop this item".
///
/// `matcher` is the shared skim matcher (fuzzy tier only).
pub fn score(matcher: &SkimMatcherV2, name: &str, keywords: &str, query: &str) -> Option<i64> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Some(IDLE);
    }
    let n = name.trim().to_lowercase();
    if n.is_empty() {
        return keyword_score(matcher, keywords, &q);
    }

    // Shorter names win ties inside a tier: 100 points for a 1-char name down to
    // 0 for anything 100 chars or longer.
    let brevity = 100i64.saturating_sub(n.chars().count() as i64).max(0);

    if n == q {
        return Some(EXACT + brevity);
    }
    if n.starts_with(&q) {
        return Some(PREFIX + brevity);
    }
    if words(&n).any(|w| w.starts_with(&q)) {
        return Some(WORD + brevity);
    }
    if !q.contains(' ') && acronym(&n).starts_with(&q) {
        return Some(ACRONYM + brevity);
    }
    if n.contains(&q) {
        return Some(SUBSTR + brevity);
    }
    // Multi-word query: every word must land somewhere in the name ("code studio").
    if q.contains(' ') && q.split_whitespace().all(|t| n.contains(t)) {
        return Some(SUBSTR + brevity);
    }
    if let Some(s) = matcher.fuzzy_match(&n, &q) {
        return Some(FUZZY + s.min(400) + brevity / 2);
    }
    keyword_score(matcher, keywords, &q)
}

/// Convenience for providers whose haystack is just the title.
pub fn score_name(matcher: &SkimMatcherV2, name: &str, query: &str) -> Option<i64> {
    score(matcher, name, "", query)
}

fn keyword_score(matcher: &SkimMatcherV2, keywords: &str, q: &str) -> Option<i64> {
    if keywords.trim().is_empty() {
        return None;
    }
    let k = keywords.to_lowercase();
    if words(&k).any(|w| w == q) {
        return Some(KEYWORD + 300);
    }
    if words(&k).any(|w| w.starts_with(q)) {
        return Some(KEYWORD + 200);
    }
    if k.contains(q) {
        return Some(KEYWORD + 100);
    }
    matcher.fuzzy_match(&k, q).map(|s| KEYWORD + s.min(90) / 2)
}

/// Split on anything that isn't alphanumeric, dropping empties.
fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty())
}

/// First letter of every word ("Visual Studio Code" → "vsc").
fn acronym(s: &str) -> String {
    words(s).filter_map(|w| w.chars().next()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str, kw: &str, q: &str) -> Option<i64> {
        score(&matcher(), name, kw, q)
    }

    #[test]
    fn prefix_beats_substring_and_fuzzy() {
        // The bug this module exists for: "si" must put Signal on top.
        let signal = s("Signal", "chat messenger", "si").unwrap();
        let extmgr = s("Extension Manager", "gnome shell", "si").unwrap();
        let assistant = s("Qt Assistant", "qt docs", "si").unwrap();
        assert!(signal > extmgr, "signal={signal} extmgr={extmgr}");
        assert!(signal > assistant, "signal={signal} assistant={assistant}");
    }

    #[test]
    fn tiers_are_ordered() {
        let exact = s("code", "", "code").unwrap();
        let prefix = s("codeblocks", "", "code").unwrap();
        let word = s("Visual Studio Code", "", "code").unwrap();
        let substr = s("xcodeless", "", "code").unwrap();
        assert!(exact > prefix && prefix > word && word > substr);
    }

    #[test]
    fn acronym_matches_initials() {
        let vsc = s("Visual Studio Code", "", "vsc").unwrap();
        assert!(vsc >= ACRONYM && vsc < WORD);
    }

    #[test]
    fn usage_boost_cannot_jump_a_tier() {
        // A heavily-used substring match must still lose to a fresh word match.
        let substr = s("xcodeless", "", "code").unwrap() + BOOST_MAX;
        let word = s("Visual Studio Code", "", "code").unwrap();
        assert!(word > substr, "word={word} boosted_substr={substr}");
    }

    #[test]
    fn keywords_are_a_last_resort() {
        let by_kw = s("Files", "nautilus browser", "nautilus").unwrap();
        let by_name = s("Nautilus", "", "nautilus").unwrap();
        assert!(by_name > by_kw);
        assert!(by_kw >= KEYWORD);
    }

    #[test]
    fn shorter_names_win_ties() {
        let short = s("Kitty", "", "k").unwrap();
        let long = s("Kdenlive Video Editor", "", "k").unwrap();
        assert!(short > long);
    }

    #[test]
    fn no_match_is_none() {
        assert!(s("Firefox", "web browser", "zzzz").is_none());
    }

    #[test]
    fn empty_query_is_idle_baseline() {
        assert_eq!(s("Anything", "", ""), Some(IDLE));
    }

    #[test]
    fn multi_word_query_matches_out_of_order() {
        assert!(s("Visual Studio Code", "", "code studio").is_some());
    }
}
