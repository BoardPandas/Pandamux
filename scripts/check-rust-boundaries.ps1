$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$cratesRoot = Join-Path $repoRoot 'crates'

$rules = @(
  @{
    Name = 'pandamux-core'
    Forbidden = @('iced', 'alacritty_terminal')
  },
  @{
    Name = 'pandamux-term'
    Forbidden = @('iced')
  },
  @{
    Name = 'pandamux-remote'
    Forbidden = @('iced', 'alacritty_terminal')
  },
  @{
    Name = 'pandamux-protocol'
    Forbidden = @('iced', 'alacritty_terminal', 'pandamux-term', 'pandamux-remote')
  },
  @{
    Name = 'pandamux-client'
    Forbidden = @('iced', 'alacritty_terminal', 'pandamux-term', 'pandamux-remote')
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
