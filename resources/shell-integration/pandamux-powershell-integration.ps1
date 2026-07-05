# pandamux PowerShell Integration
# Injected automatically by pandamux

$env:PANDAMUX = "1"

# Named pipe client helper
function Send-PandaMUXMessage {
    param([string]$Message)
    try {
        $pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", "pandamux", [System.IO.Pipes.PipeDirection]::InOut)
        $pipe.Connect(1000)
        $writer = New-Object System.IO.StreamWriter($pipe)
        $writer.AutoFlush = $true
        $writer.WriteLine($Message)
        $pipe.Close()
    } catch {
        # Silently ignore pipe errors
    }
}

# Report CWD
function Report-Cwd {
    $surfaceId = $env:PANDAMUX_SURFACE_ID
    if ($surfaceId) {
        Send-PandaMUXMessage "report_pwd $surfaceId $PWD"
    }
}

# Report git branch
function Report-GitBranch {
    $surfaceId = $env:PANDAMUX_SURFACE_ID
    if (-not $surfaceId) { return }

    try {
        $branch = git rev-parse --abbrev-ref HEAD 2>$null
        if ($LASTEXITCODE -eq 0 -and $branch) {
            $dirty = ""
            $status = git status --porcelain 2>$null
            if ($status) { $dirty = "dirty" }
            Send-PandaMUXMessage "report_git_branch $surfaceId $branch $dirty"
        } else {
            Send-PandaMUXMessage "clear_git_branch $surfaceId"
        }
    } catch {
        Send-PandaMUXMessage "clear_git_branch $surfaceId"
    }
}

# Report shell state
function Report-ShellState {
    param([string]$State)
    $surfaceId = $env:PANDAMUX_SURFACE_ID
    if ($surfaceId) {
        Send-PandaMUXMessage "report_shell_state $surfaceId $State"
    }
}

# Report "running" when user executes a command (pre-execution hook)
if (Get-Module -Name PSReadLine -ErrorAction SilentlyContinue) {
    Set-PSReadLineKeyHandler -Key Enter -ScriptBlock {
        # Report running state before the command executes
        Report-ShellState "running"
        # Accept the line (execute the command)
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
    }
}

# Override prompt (fires AFTER command completes)
$_pandamux_original_prompt = $function:prompt
function prompt {
    Report-Cwd
    Report-GitBranch
    # Detect if last command was interrupted (Ctrl+C → exit code -1073741510 on Windows)
    if ($LASTEXITCODE -eq -1073741510 -or $LASTEXITCODE -eq 130) {
        Report-ShellState "interrupted"
    } else {
        Report-ShellState "idle"
    }
    Send-PandaMUXMessage "ports_kick $env:PANDAMUX_SURFACE_ID"

    # Call original prompt or default
    if ($_pandamux_original_prompt) {
        & $_pandamux_original_prompt
    } else {
        "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) "
    }
}

# PR polling background job (every 45 seconds)
$_pandamux_pr_job = Start-Job -ScriptBlock {
    param($surfaceId, $pipeName)
    while ($true) {
        Start-Sleep -Seconds 45
        try {
            $prJson = gh pr view --json number,state,title 2>$null
            if ($LASTEXITCODE -eq 0 -and $prJson) {
                $pr = $prJson | ConvertFrom-Json
                $pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", $pipeName, [System.IO.Pipes.PipeDirection]::InOut)
                $pipe.Connect(1000)
                $writer = New-Object System.IO.StreamWriter($pipe)
                $writer.AutoFlush = $true
                $writer.WriteLine("report_pr $surfaceId $($pr.number) $($pr.state) $($pr.title)")
                $pipe.Close()
            }
        } catch { }
    }
} -ArgumentList $env:PANDAMUX_SURFACE_ID, "pandamux"
