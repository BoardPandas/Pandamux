param(
    [string]$TargetDir = "target/release"
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$binDir = Join-Path $repoRoot $TargetDir

$exeSuffix = if ($IsWindows -or $env:OS -eq 'Windows_NT') { '.exe' } else { '' }
$serverBin = Join-Path $binDir ("pandamux-server$exeSuffix")
$cliBin = Join-Path $binDir ("pandamux-cli$exeSuffix")

if (-not (Test-Path -LiteralPath $serverBin)) {
    # Fallback to target/debug if release does not exist
    $debugDir = Join-Path $repoRoot "target/debug"
    $serverBin = Join-Path $debugDir ("pandamux-server$exeSuffix")
    $cliBin = Join-Path $debugDir ("pandamux-cli$exeSuffix")
}

if (-not (Test-Path -LiteralPath $serverBin)) {
    throw "pandamux-server binary not found at $serverBin"
}
if (-not (Test-Path -LiteralPath $cliBin)) {
    throw "pandamux-cli binary not found at $cliBin"
}

Write-Host "Evaluating built artifacts (BP evaluate-built-artifacts-in-ci):"
Write-Host "  Server: $serverBin"
Write-Host "  CLI:    $cliBin"

# Spawn pandamux-server --headless in background
$serverProc = Start-Process -FilePath $serverBin -ArgumentList "--headless" -PassThru -NoNewWindow
Write-Host "Spawned server PID $($serverProc.Id)"

try {
    # Wait for server runtime info to become ready
    $ready = $false
    $timeoutSeconds = 15
    $startTime = [System.DateTime]::UtcNow

    while (([System.DateTime]::UtcNow - $startTime).TotalSeconds -lt $timeoutSeconds) {
        if ($serverProc.HasExited) {
            throw "Server exited prematurely with exit code $($serverProc.ExitCode)"
        }

        # Try running cli hello
        $testResult = & $cliBin hello --json 2>&1 | Out-String
        if ($LASTEXITCODE -eq 0 -and $testResult -match '"protocolVersion"') {
            $ready = $true
            Write-Host "Server responded to handshake in $([Math]::Round(([System.DateTime]::UtcNow - $startTime).TotalMilliseconds))ms"
            break
        }
        Start-Sleep -Milliseconds 250
    }

    if (-not $ready) {
        throw "Timed out waiting for server to respond to system.hello handshake ($timeoutSeconds s)"
    }

    # Test 1: system.hello handshake
    Write-Host "`n1. Testing system.hello handshake:"
    $helloOutput = & $cliBin hello --json
    if ($LASTEXITCODE -ne 0) {
        throw "pandamux-cli hello failed with exit code $LASTEXITCODE"
    }
    Write-Host $helloOutput

    # Test 2: system.ping connectivity and latency
    Write-Host "`n2. Testing system.ping:"
    $pingOutput = & $cliBin ping --json
    if ($LASTEXITCODE -ne 0) {
        throw "pandamux-cli ping failed with exit code $LASTEXITCODE"
    }
    Write-Host $pingOutput

    # Test 3: system.identify metadata
    Write-Host "`n3. Testing system.identify:"
    $identifyOutput = & $cliBin identify --json
    if ($LASTEXITCODE -ne 0) {
        throw "pandamux-cli identify failed with exit code $LASTEXITCODE"
    }
    Write-Host $identifyOutput

    Write-Host "`nArtifact smoke test PASSED: server and CLI communicate cleanly over IPC."
}
finally {
    if ($serverProc -and -not $serverProc.HasExited) {
        Write-Host "Stopping background server PID $($serverProc.Id)..."
        Stop-Process -Id $serverProc.Id -Force -ErrorAction SilentlyContinue
    }
}
