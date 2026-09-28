$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$targetDir = Join-Path $repoRoot 'target'
$outputDir = Join-Path $repoRoot (Join-Path 'resources' 'remote-binaries')

if (-not (Test-Path -LiteralPath $outputDir)) {
  New-Item -ItemType Directory -Path $outputDir -Force | Out-Null
}

$targets = @('x86_64-unknown-linux-musl', 'aarch64-unknown-linux-musl')

Write-Host 'Building Linux musl binaries for remote environments...'

foreach ($target in $targets) {
  Write-Host "Target: $target"
  $hasCross = Get-Command cross -ErrorAction SilentlyContinue
  if ($hasCross) {
    & cross build --release --target $target -p pandamux-server -p pandamux-cli
  } else {
    try {
      & cargo build --release --target $target -p pandamux-server -p pandamux-cli
    } catch {
      Write-Warning "Direct cargo build for $target requires cross-compilation toolchain or cross."
    }
  }

  $archDir = Join-Path $outputDir $target
  if (-not (Test-Path -LiteralPath $archDir)) {
    New-Item -ItemType Directory -Path $archDir -Force | Out-Null
  }

  foreach ($bin in @('pandamux-server', 'pandamux')) {
    $src = Join-Path (Join-Path (Join-Path $targetDir $target) 'release') $bin
    if (Test-Path -LiteralPath $src) {
      $dest = Join-Path $archDir $bin
      Copy-Item -LiteralPath $src -Destination $dest -Force
      $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $src).Hash.ToLowerInvariant()
      $size = (Get-Item -LiteralPath $src).Length
      Write-Host "  ${bin}: sha256=${hash} size=${size} bytes"
    }
  }
}

Write-Host 'Remote musl build script finished.'
