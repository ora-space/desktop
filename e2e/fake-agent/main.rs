//! Deterministic agent plugin process used by Desktop E2E tests in place of Deno and OpenCode.

mod acp;
mod journal;
mod plugin;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    plugin::run().await
}
