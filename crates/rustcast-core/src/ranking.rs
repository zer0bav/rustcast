//! Shared fuzzy matcher factory. Frecency is a later refinement (Phase 4).

use fuzzy_matcher::skim::SkimMatcherV2;

pub fn matcher() -> SkimMatcherV2 {
    SkimMatcherV2::default()
}
