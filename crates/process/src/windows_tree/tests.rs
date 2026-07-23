use std::io;
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use tokio::io::AsyncReadExt;

use super::*;

fn cmd_executable() -> io::Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("SystemRoot")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SystemRoot is not set"))?,
    )
    .join("System32")
    .join("cmd.exe"))
}

#[test]
fn command_line_quoting_preserves_backslashes_and_quotes() -> Result<(), Box<dyn std::error::Error>>
{
    let spec = ProcessSpec::new(r"C:\Program Files\Bun\bun.exe")
        .arg("plain")
        .arg("two words")
        .arg(r#"quote\"inside"#)
        .arg(r"ends with slash\");

    let encoded = build_command_line(&spec)?;
    let rendered = String::from_utf16(&encoded[..encoded.len() - 1])?;
    assert_eq!(
        rendered,
        r#""C:\Program Files\Bun\bun.exe" plain "two words" "quote\\\"inside" "ends with slash\\""#
    );
    Ok(())
}

#[tokio::test]
async fn child_is_contained_at_creation_and_host_observes_pipe_eof()
-> Result<(), Box<dyn std::error::Error>> {
    let system_root = std::env::var_os("SystemRoot")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SystemRoot is not set"))?;
    let tree = WindowsJobProcessTreeSpawner::new().spawn_tree(
        ProcessSpec::new(cmd_executable()?)
            .args(["/D", "/S", "/C", "echo hello"])
            .clear_and_allowlist_environment()
            .env("SystemRoot", system_root),
    )?;
    let parts = tree.into_parts()?;
    let mut stdout = parts.stdio.stdout;
    drop(parts.stdio.stdin);
    let mut output = Vec::new();
    stdout.read_to_end(&mut output).await?;

    let exit = parts.direct_exit.await?;
    parts.tree_empty.await?;
    assert_eq!(
        exit,
        ProcessExit {
            exit_code: Some(0),
            success: true
        }
    );
    assert_eq!(String::from_utf8(output)?.trim(), "hello");
    Ok(())
}

#[tokio::test]
async fn terminate_tree_is_idempotent_and_waits_for_descendants()
-> Result<(), Box<dyn std::error::Error>> {
    let tree = WindowsJobProcessTreeSpawner::new().spawn_tree(
        ProcessSpec::new(cmd_executable()?).args([
            "/D",
            "/S",
            "/C",
            "start \"\" /B ping.exe -n 30 127.0.0.1 > nul & ping.exe -n 30 127.0.0.1 > nul",
        ]),
    )?;
    let parts = tree.into_parts()?;

    parts.controller.terminate_tree()?;
    parts.controller.terminate_tree()?;
    let exit = parts.direct_exit.await?;
    parts.tree_empty.await?;
    assert!(!exit.success);
    Ok(())
}
