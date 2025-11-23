pub mod server;
pub mod proto;
pub mod config;

use crate::agent::AgentState;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// Main entry point for the CNPG plugin
pub async fn run_cnpg_plugin(port: u16, config_path: Option<PathBuf>) -> anyhow::Result<()> {
    info!("Starting Vaanyze CNPG Plugin on port {}", port);
    
    // Load configuration
    let config = config::load_plugin_config(config_path)?;
    
    // Initialize state
    let state = Arc::new(Mutex::new(AgentState::default()));
    
    // Start gRPC server
    server::start_grpc_server(port, config, state).await?;
    
    Ok(())
}
