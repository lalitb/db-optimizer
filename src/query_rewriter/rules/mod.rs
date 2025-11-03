mod limit_adder;
mod select_star_remover;
mod predicate_pushdown;
mod subquery_flattener;
mod redundant_join_remover;

// Enhanced versions with full rewriting
mod predicate_pushdown_enhanced;
mod subquery_flattener_enhanced;
mod select_star_remover_enhanced;
mod redundant_join_remover_enhanced;

pub use limit_adder::LimitAdder;
pub use select_star_remover::SelectStarRemover;
pub use predicate_pushdown::PredicatePushdown;
pub use subquery_flattener::SubqueryFlattener;
pub use redundant_join_remover::RedundantJoinRemover;

// Enhanced versions
pub use predicate_pushdown_enhanced::PredicatePushdownEnhanced;
pub use subquery_flattener_enhanced::SubqueryFlattenerEnhanced;
pub use select_star_remover_enhanced::SelectStarRemoverEnhanced;
pub use redundant_join_remover_enhanced::RedundantJoinRemoverEnhanced;
