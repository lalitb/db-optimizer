mod limit_adder;
mod predicate_pushdown;
mod redundant_join_remover;
mod select_star_remover;
mod subquery_flattener;

// Enhanced versions with full rewriting
mod predicate_pushdown_enhanced;
mod redundant_join_remover_enhanced;
mod select_star_remover_enhanced;
mod subquery_flattener_enhanced;

pub use limit_adder::LimitAdder;
pub use predicate_pushdown::PredicatePushdown;
pub use redundant_join_remover::RedundantJoinRemover;
pub use select_star_remover::SelectStarRemover;
pub use subquery_flattener::SubqueryFlattener;

// Enhanced versions
pub use predicate_pushdown_enhanced::PredicatePushdownEnhanced;
pub use redundant_join_remover_enhanced::RedundantJoinRemoverEnhanced;
pub use select_star_remover_enhanced::SelectStarRemoverEnhanced;
pub use subquery_flattener_enhanced::SubqueryFlattenerEnhanced;
