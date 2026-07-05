# pandamux PowerShell Integration
# Injected automatically by pandamux

$env:PANDAMUX = "1"

# UTF-8 I/O so multi-byte input (Korean, Japanese, Chinese, emoji, accents)
# survives the conpty round-trip cleanly.
try {
    [Console]::InputEncoding = [System.Text.UTF8Encoding]::new()
    [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
    $OutputEncoding = [System.Text.UTF8Encoding]::new()
    $PSDefaultParameterValues['Out-File:Encoding'] = 'utf8'
} catch {}

# pandamux CLI shortcut — Claude Code and users can just type: pandamux browser open <url>
function pandamux { node "$env:PANDAMUX_CLI" @args }

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

# PR polling background job (every 45 seconds).
# DEFERRED: Start-Job spins up a whole child PowerShell runspace and costs
# several hundred ms — running it during init delayed the FIRST prompt. We
# instead start it on the shell's first idle (after the prompt is already on
# screen), so it never sits on the startup critical path. A global guard makes it
# fire exactly once; PR data isn't needed in the first 45s anyway.
$global:_pandamux_pr_started = $false
$null = Register-EngineEvent -SourceIdentifier ([System.Management.Automation.PSEngineEvent]::OnIdle) -Action {
    if ($global:_pandamux_pr_started) { return }
    $global:_pandamux_pr_started = $true
    $global:_pandamux_pr_job = Start-Job -ScriptBlock {
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
}

# Quick-launch profile startup commands (issue #32).
# pandamux passes these in PANDAMUX_STARTUP_COMMANDS (newline-separated) so they run as
# part of init — before the first interactive prompt — rather than being injected
# as keystrokes afterward. Keystroke injection raced the shell's init-time
# Device Attributes query (ConPTY answers DA1 with "\e[?62;4;9;22c" on stdin);
# when that response landed on the prompt alongside an injected "<cmd>\r" the two
# merged into a bogus executed line (e.g. "62;4;9;22ccls"). Running here avoids
# that entirely. Runs last so the prompt override / PSReadLine handlers exist.
if ($env:PANDAMUX_STARTUP_COMMANDS) {
    foreach ($_pandamux_cmd in ($env:PANDAMUX_STARTUP_COMMANDS -split "`n")) {
        $_pandamux_cmd = $_pandamux_cmd.Trim()
        if ($_pandamux_cmd) {
            try { Invoke-Expression $_pandamux_cmd } catch { Write-Error $_ }
        }
    }
    # One-shot: don't let it leak into child shells spawned from this session.
    Remove-Item Env:\PANDAMUX_STARTUP_COMMANDS -ErrorAction SilentlyContinue
}
