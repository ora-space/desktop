param([string]$Port = "8090")

# Logdy 0.17.1 can miss NTFS append notifications, so let PowerShell own the file watch.
$logs = @(Get-ChildItem -Path ".data/logs/ora.log.*" -File | Sort-Object Name)
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
Get-Content -LiteralPath $logs[-1].FullName -Encoding UTF8 -Wait |
    logdy --ui-ip 127.0.0.1 --port $Port --no-analytics --no-updates --config logdy.config.json
exit $LASTEXITCODE
