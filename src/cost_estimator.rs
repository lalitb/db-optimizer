use crate::models::*;

pub struct CostEstimator {
    /// Cost per hour of database compute (AWS RDS db.t3.medium ~$0.07/hr)
    pub cost_per_hour_usd: f64,
}

impl CostEstimator {
    pub fn new() -> Self {
        Self {
            cost_per_hour_usd: 0.07, // Default: AWS RDS db.t3.medium
        }
    }
    
    pub fn with_cost_per_hour(mut self, cost: f64) -> Self {
        self.cost_per_hour_usd = cost;
        self
    }
    
    /// Estimate monthly cost savings for a recommendation
    pub fn estimate_savings(&self, rec: &mut Recommendation) {
        let monthly_savings = match rec.issue_type {
            IssueType::MissingIndex => {
                self.estimate_index_savings(
                    rec.estimated_time_savings_ms,
                    rec.affected_queries_count,
                )
            }
            IssueType::UnusedIndex => {
                // Already calculated in analyzer (storage cost)
                rec.estimated_cost_savings_usd_monthly.unwrap_or(0.0)
            }
            IssueType::SlowQuery => {
                self.estimate_query_optimization_savings(
                    rec.estimated_time_savings_ms,
                    rec.affected_queries_count,
                )
            }
            IssueType::HighVolumeQuery => {
                if let Some(time_ms) = rec.estimated_time_savings_ms {
                    if let Some(calls) = rec.affected_queries_count {
                        let hours_saved = (time_ms * calls as f64) / (1000.0 * 60.0 * 60.0);
                        hours_saved * self.cost_per_hour_usd * 30.0 // Monthly
                    } else {
                        0.0
                    }
                } else {
                    0.0
                }
            }
        };
        
        rec.estimated_cost_savings_usd_monthly = Some(monthly_savings.max(0.0));
    }
    
    fn estimate_index_savings(
        &self,
        time_savings_ms: Option<f64>,
        query_count: Option<i64>,
    ) -> f64 {
        match (time_savings_ms, query_count) {
            (Some(time_ms), Some(count)) => {
                // Assume queries run uniformly throughout the month
                let daily_queries = count; // From pg_stat_statements (since last reset)
                
                // Rough estimate: extrapolate to 30 days
                let monthly_time_saved_hours = 
                    (time_ms * daily_queries as f64 * 30.0) / (1000.0 * 60.0 * 60.0);
                
                monthly_time_saved_hours * self.cost_per_hour_usd
            }
            _ => 0.0,
        }
    }
    
    fn estimate_query_optimization_savings(
        &self,
        time_savings_ms: Option<f64>,
        query_count: Option<i64>,
    ) -> f64 {
        // Similar calculation to index savings but more conservative
        self.estimate_index_savings(time_savings_ms, query_count) * 0.5
    }
}

impl Default for CostEstimator {
    fn default() -> Self {
        Self::new()
    }
}
