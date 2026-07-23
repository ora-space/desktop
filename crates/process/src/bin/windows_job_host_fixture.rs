#[cfg(windows)]
use std::io::{self, Write};

#[cfg(windows)]
use ora_process::{
    ManagedProcessTree, ProcessSpec, ProcessTreeSpawner, WindowsJobProcessTreeSpawner,
};
#[cfg(windows)]
use tokio::io::{AsyncBufReadExt, BufReader};

#[cfg(windows)]
#[tokio::main]
async fn main() -> io::Result<()> {
    let powershell = std::path::PathBuf::from(
        std::env::var_os("SystemRoot")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SystemRoot is not set"))?,
    )
    .join("System32")
    .join("WindowsPowerShell")
    .join("v1.0")
    .join("powershell.exe");
    let tree = WindowsJobProcessTreeSpawner::new().spawn_tree(ProcessSpec::new(powershell).args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "$child = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\\ping.exe') -ArgumentList @('-n', '30', '127.0.0.1') -WindowStyle Hidden -PassThru; [Console]::Out.WriteLine($child.Id); Start-Sleep -Seconds 30",
    ]))?;
    let direct_process_id = tree.direct_process_id();
    let parts = tree.into_parts().map_err(io::Error::other)?;
    drop(parts.stdio.stdin);

    let mut descendant_process_id = String::new();
    BufReader::new(parts.stdio.stdout)
        .read_line(&mut descendant_process_id)
        .await?;
    println!("{direct_process_id} {}", descendant_process_id.trim());
    io::stdout().flush()?;

    // The parent opens synchronization handles before acknowledging this line. Exiting without
    // unwinding models an abrupt Host failure and leaves Windows to enforce KILL_ON_JOB_CLOSE.
    let mut acknowledgement = String::new();
    io::stdin().read_line(&mut acknowledgement)?;
    std::process::exit(23);
}

#[cfg(not(windows))]
fn main() {}
