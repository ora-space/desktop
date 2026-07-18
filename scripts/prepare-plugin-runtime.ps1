[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$workspace = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$sourcePath = Join-Path $workspace 'runtime-assets\bun-source-v1.json'
$source = Get-Content -LiteralPath $sourcePath -Raw | ConvertFrom-Json

if ($source.schemaVersion -ne 1 -or
    $source.target -ne 'x86_64-pc-windows-msvc' -or
    $source.artifact -ne 'bun-windows-x64-baseline.zip') {
    throw 'Unsupported Bun source manifest.'
}

$expectedUrl = "https://github.com/oven-sh/bun/releases/download/bun-v$($source.version)/$($source.artifact)"
if ($source.officialUrl -cne $expectedUrl -or $source.sha256 -notmatch '^[0-9a-f]{64}$') {
    throw 'Bun source identity is not the exact approved official artifact.'
}

$cacheRoot = Join-Path $workspace ".cache\plugin-runtime\bun-v$($source.version)"
$archivePath = Join-Path $cacheRoot $source.artifact
$bunPath = Join-Path $cacheRoot 'bun.exe'
New-Item -ItemType Directory -Path $cacheRoot -Force | Out-Null

function Test-ApprovedArchive {
    if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
        return $false
    }
    $file = Get-Item -LiteralPath $archivePath
    if ($file.Length -ne [int64]$source.bytes) {
        return $false
    }
    $digest = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    return $digest -ceq $source.sha256
}

if (-not (Test-ApprovedArchive)) {
    $downloadPath = Join-Path $cacheRoot ("download-{0}.tmp" -f [guid]::NewGuid())
    Invoke-WebRequest -Uri $source.officialUrl -OutFile $downloadPath -UseBasicParsing
    $download = Get-Item -LiteralPath $downloadPath
    $downloadDigest = (Get-FileHash -LiteralPath $downloadPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($download.Length -ne [int64]$source.bytes -or $downloadDigest -cne $source.sha256) {
        Remove-Item -LiteralPath $downloadPath -Force
        throw 'Downloaded Bun archive does not match the approved size and SHA-256.'
    }
    Move-Item -LiteralPath $downloadPath -Destination $archivePath -Force
}

if (-not (Test-Path -LiteralPath $bunPath -PathType Leaf)) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($archivePath)
    try {
        $entries = @($archive.Entries | Where-Object { $_.FullName -match '(^|/)bun\.exe$' })
        if ($entries.Count -ne 1 -or $entries[0].Length -le 0) {
            throw 'Approved Bun archive does not contain exactly one bun.exe.'
        }
        $temporaryBun = Join-Path $cacheRoot ("bun-{0}.tmp" -f [guid]::NewGuid())
        $input = $entries[0].Open()
        $output = [System.IO.File]::Open($temporaryBun, [System.IO.FileMode]::CreateNew)
        try {
            $input.CopyTo($output)
            $output.Flush($true)
        }
        finally {
            $output.Dispose()
            $input.Dispose()
        }
        Move-Item -LiteralPath $temporaryBun -Destination $bunPath
    }
    finally {
        $archive.Dispose()
    }
}

$outputRoot = Join-Path $workspace 'runtime-assets\prepared'
$stagingRoot = Join-Path $workspace ("runtime-assets\.prepared-{0}" -f [guid]::NewGuid())
New-Item -ItemType Directory -Path $stagingRoot | Out-Null
try {
    $bundlePath = Join-Path $stagingRoot 'plugin-host-bootstrap.js'
    $entryPath = Join-Path $workspace 'packages\plugin-runtime\src\bootstrap-entry.ts'
    $emptyBunfig = Join-Path $workspace 'runtime-assets\empty-bunfig.toml'
    $configArgument = "--config=$emptyBunfig"
    & $bunPath $configArgument --no-env-file --no-macros build $entryPath `
        --target=bun --format=esm --packages=bundle --outfile=$bundlePath --minify
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $bundlePath -PathType Leaf)) {
        throw 'Pinned Bun failed to build the private bootstrap bundle.'
    }

    Copy-Item -LiteralPath $bunPath -Destination (Join-Path $stagingRoot 'bun.exe')
    Copy-Item -LiteralPath $emptyBunfig `
        -Destination (Join-Path $stagingRoot 'empty-bunfig.toml')

    $roles = @(
        @{ role = 'bunExecutable'; path = 'bun.exe' },
        @{ role = 'bootstrapBundle'; path = 'plugin-host-bootstrap.js' },
        @{ role = 'emptyBunfig'; path = 'empty-bunfig.toml' }
    )
    $files = foreach ($role in $roles) {
        $path = Join-Path $stagingRoot $role.path
        $file = Get-Item -LiteralPath $path
        $digest = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        [ordered]@{
            role = $role.role
            path = $role.path
            digest = "sha256:$digest"
            bytes = $file.Length
        }
    }
    $manifest = [ordered]@{
        schemaVersion = 1
        target = 'x86_64-pc-windows-msvc'
        runtimeVersion = '1.0.0'
        wireVersion = 1
        pluginApi = 1
        bootstrapSchemaVersion = 1
        configSchemaVersion = 1
        bun = [ordered]@{
            version = $source.version
            officialUrl = $source.officialUrl
            archiveDigest = "sha256:$($source.sha256)"
            archiveBytes = [int64]$source.bytes
        }
        files = @($files)
    }
    $manifestBytes = $manifest | ConvertTo-Json -Depth 8
    [System.IO.File]::WriteAllText(
        (Join-Path $stagingRoot 'runtime-manifest.json'),
        $manifestBytes,
        [System.Text.UTF8Encoding]::new($false)
    )

    $expectedOutputPrefix = [System.IO.Path]::GetFullPath((Join-Path $workspace 'runtime-assets')) + [System.IO.Path]::DirectorySeparatorChar
    $resolvedOutput = [System.IO.Path]::GetFullPath($outputRoot)
    if (-not $resolvedOutput.StartsWith($expectedOutputPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Prepared runtime output escaped the managed runtime-assets directory.'
    }
    if (Test-Path -LiteralPath $outputRoot) {
        Remove-Item -LiteralPath $outputRoot -Recurse -Force
    }
    Move-Item -LiteralPath $stagingRoot -Destination $outputRoot
}
finally {
    if (Test-Path -LiteralPath $stagingRoot) {
        Remove-Item -LiteralPath $stagingRoot -Recurse -Force
    }
}

Write-Output "Prepared pinned plugin runtime resources at $outputRoot"
