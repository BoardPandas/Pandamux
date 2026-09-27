$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$cratesRoot = Join-Path $repoRoot 'crates'

$rules = @(
  @{
    Name = 'pandamux-core'
    Forbidden = @('iced', 'alacritty_terminal', 'rusqlite')
  },
  @{
    Name = 'pandamux-term'
    Forbidden = @('iced', 'rusqlite')
  },
  @{
    Name = 'pandamux-remote'
    Forbidden = @('iced', 'alacritty_terminal', 'rusqlite')
  },
  @{
    Name = 'pandamux-protocol'
    Forbidden = @('iced', 'alacritty_terminal', 'pandamux-term', 'pandamux-remote', 'rusqlite')
  },
  @{
    Name = 'pandamux-client'
    Forbidden = @('iced', 'alacritty_terminal', 'pandamux-term', 'pandamux-remote', 'rusqlite')
  },
  @{
    Name = 'pandamux-server'
    Forbidden = @('iced', 'tauri', 'alacritty_terminal', 'portable-pty', 'russh')
  },
  @{
    Name = 'pandamux-providers'
    Forbidden = @('iced', 'tauri', 'alacritty_terminal', 'portable-pty', 'russh', 'rusqlite')
  }
)

foreach ($rule in $rules) {
  $manifest = Join-Path $cratesRoot (Join-Path $rule.Name 'Cargo.toml')
  if (-not (Test-Path -LiteralPath $manifest)) {
    throw "Missing manifest: $manifest"
  }

  $content = Get-Content -Raw -LiteralPath $manifest
  foreach ($forbidden in @($rule.Forbidden)) {
    if ($content -match "(?m)^\s*$([regex]::Escape($forbidden))\s*=") {
      throw "$($rule.Name) must not depend on $forbidden"
    }
  }
}

Write-Host 'Rust crate boundary check passed.'
