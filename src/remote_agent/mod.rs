pub mod models;
pub mod client;
// server.rs would be included if we were building the agent binary, 
// but for the library we mainly need the client and models.
// We will include a mock server for testing/demonstration.
pub mod server; 
