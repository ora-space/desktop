#![cfg(unix)]

use ora_process::{
    ManagedProcess, ProcessEnvironment, ProcessSpawner, ProcessSpec, TokioProcessSpawner,
};
use pretty_assertions::assert_eq;
use tokio::io::AsyncReadExt;

/// A real process receives only explicitly supplied variables when its owner clears inheritance.
#[tokio::test]
async fn empty_environment_does_not_inherit_host_authority()
-> Result<(), Box<dyn std::error::Error>> {
    let spawner = TokioProcessSpawner::new().with_environment(ProcessEnvironment::Empty);
    let mut process =
        spawner.spawn(ProcessSpec::new("/usr/bin/env").env("CIRCUS_VISIBLE", "explicit"))?;
    let mut stdout = process.take_stdout().ok_or("stdout unavailable")?;
    let mut output = String::new();
    stdout.read_to_string(&mut output).await?;
    assert!(process.wait().await?.success());
    assert_eq!(output, "CIRCUS_VISIBLE=explicit\n");
    Ok(())
}
