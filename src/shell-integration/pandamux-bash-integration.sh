#!/bin/bash
# pandamux Bash/Zsh Integration
# Sourced via PANDAMUX_INTEGRATION=1 detection

export PANDAMUX=1

# pandamux CLI shortcut — Claude Code and users can just type: pandamux browser open <url>
pandamux() { node "$PANDAMUX_CLI" "$@"; }
export -f pandamux

_pandamux_report() {
    local msg="$1"
    # Write to temp file for main process to pick up
    local tmpdir="/mnt/c/Users/${USER}/AppData/Local/Temp/pandamux"
    mkdir -p "$tmpdir" 2>/dev/null
    echo "$msg" >> "$tmpdir/messages"
}

_pandamux_report_cwd() {
    local surface_id="${PANDAMUX_SURFACE_ID}"
    [ -z "$surface_id" ] && return
    _pandamux_report "report_pwd $surface_id $(pwd)"
}

_pandamux_report_git() {
    local surface_id="${PANDAMUX_SURFACE_ID}"
    [ -z "$surface_id" ] && return
    local branch
    branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null)
    if [ $? -eq 0 ] && [ -n "$branch" ]; then
        local dirty=""
        [ -n "$(git status --porcelain 2>/dev/null)" ] && dirty="dirty"
        _pandamux_report "report_git_branch $surface_id $branch $dirty"
    else
        _pandamux_report "clear_git_branch $surface_id"
    fi
}

_pandamux_precmd() {
    local exit_code=$?
    _pandamux_report_cwd
    _pandamux_report_git
    # 130 = SIGINT (Ctrl+C), 137 = SIGKILL, 143 = SIGTERM
    if [ $exit_code -eq 130 ] || [ $exit_code -eq 137 ] || [ $exit_code -eq 143 ]; then
        _pandamux_report "report_shell_state ${PANDAMUX_SURFACE_ID} interrupted"
    else
        _pandamux_report "report_shell_state ${PANDAMUX_SURFACE_ID} idle"
    fi
    _pandamux_report "ports_kick ${PANDAMUX_SURFACE_ID}"
}

# Report "running" before a command executes (pre-execution hook)
_pandamux_preexec() {
    local surface_id="${PANDAMUX_SURFACE_ID}"
    [ -z "$surface_id" ] && return
    _pandamux_report "report_shell_state $surface_id running"
}

# Install hooks
if [ -n "$ZSH_VERSION" ]; then
    # Zsh: native preexec + precmd
    autoload -Uz add-zsh-hook
    add-zsh-hook precmd _pandamux_precmd
    add-zsh-hook preexec _pandamux_preexec
elif [ -n "$BASH_VERSION" ]; then
    # Bash: DEBUG trap as preexec, PROMPT_COMMAND as precmd
    _pandamux_bash_preexec_active=0
    trap '_pandamux_bash_preexec_active=1; _pandamux_preexec' DEBUG
    PROMPT_COMMAND="_pandamux_precmd; _pandamux_bash_preexec_active=0${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
fi
