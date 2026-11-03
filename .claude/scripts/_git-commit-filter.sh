#!/usr/bin/env bash
# Shared self-filter for the git-commit hooks. Source it, do not execute it:
#
#   . "$(dirname "$0")/_git-commit-filter.sh"
#   read_hook_input                 # sets HOOK_COMMAND from stdin
#   is_git_commit || exit 0
#
# Why this exists as a helper rather than four copies: the four commit hooks had
# four slightly different inline filters, and the one that drifted (a missing
# ">&2") went unnoticed for months. One definition, one place to fix.

# JSON extraction lives in _json-parser.sh for the same reason, one level down.
. "$(dirname "${BASH_SOURCE[0]}")/_json-parser.sh"

# Reads the hook's JSON payload from stdin and sets:
#   HOOK_INPUT    raw payload (kept for callers that need the unparsed text)
#   HOOK_COMMAND  tool_input.command only
#   HOOK_CWD      the payload's cwd, i.e. where the command is about to run
#
# Scanning tool_input.command rather than the whole payload matters: a PostToolUse
# payload also carries tool_response, so a command whose OUTPUT merely mentions
# "git commit" would otherwise trip the filter.
# (LL-G kb/bash/hook-scans-tool-output-false-record.md)
read_hook_input() {
  HOOK_INPUT=$(cat)
  HOOK_COMMAND=$(json_field "$HOOK_INPUT" tool_input.command)

  # Degraded path: no working parser, or a parser that returned nothing for a
  # payload that clearly carries a command. Recover the command from the raw
  # text -- over-eager, but a blocking hook must not go quiet just because a
  # parser is missing.
  #
  # json_field_flat is deliberately NOT used: it stops at the first quote, so
  # `echo "hi" && git commit` truncates to `echo ` and the gate stops blocking.
  # Under-eager is the one direction this hook cannot afford.
  #
  # Handing over the whole payload -- what this did before -- is not a fallback
  # either. is_git_commit strips quoted regions, and in raw JSON the command IS
  # a quoted region, so it strips to nothing and never matches. That fallback
  # was itself dead; it just had no way to show it while the parser branch was
  # also dead.
  if [ -z "$HOOK_COMMAND" ]; then
    HOOK_COMMAND=$(json_field_greedy "$HOOK_INPUT" command)
  fi

  # Relative paths in the command (`cd sub && git commit`) resolve against the
  # directory the command will run in, which the payload carries. Falling back to
  # the hook process's own cwd is right for a directly-invoked script and for a
  # payload written by an older harness.
  HOOK_CWD=$(json_field "$HOOK_INPUT" cwd)
  [ -n "$HOOK_CWD" ] && [ -d "$HOOK_CWD" ] || HOOK_CWD=$PWD
}

# True when HOOK_COMMAND actually invokes `git commit`.
#
#   git commit -m "mentions git commit"     -> is a commit
#   git add CHANGELOG.md && git commit      -> is a commit
#   git -C /other/repo commit               -> is a commit
#   GIT_AUTHOR_NAME=x git commit            -> is a commit
#   grep -r 'git commit' docs/              -> not a commit
#   git config --get commit.gpgsign         -> not a commit
#   echo git commit                         -> not a commit
#
# This walks the token stream rather than matching a regex, because a regex
# fails in both directions at once and cannot be tuned out of it. Tight enough
# to require `git` immediately followed by `commit` misses `git -C /repo commit`,
# since git's global flags sit between the two -- and a missed commit means the
# gate silently does not fire, the worst outcome available. Loosening it to
# allow arbitrary tokens in between starts blocking `git log --grep=commit` and
# `git config --get commit.gpgsign`. The walk gets both right by modelling what
# git's argv actually looks like.
# (LL-G kb/claude-code/hook-git-commit-filter-needs-argv-walk.md)
is_git_commit() {
  local normalized tok state result restore_glob

  # Quoted regions collapse to ONE opaque token rather than being deleted.
  # Deleting them looks equivalent and is not: `git -C "/path with space" commit`
  # would become `git -C  commit`, and the -C would then swallow the word
  # `commit` as its own value -- a commit that reports itself as not a commit.
  #
  # Newlines become separators, so the second line of a two-line script is still
  # a command position.
  normalized=$(printf '%s' "$HOOK_COMMAND" \
    | sed -e "s/'[^']*'/__CCQ__/g" -e 's/"[^"]*"/__CCQ__/g' \
    | tr '\n' ';' \
    | sed -e 's/[;&|()][;&|()]*/ ; /g')

  # Unquoted word splitting below would otherwise glob-expand a token like `*`
  # against the working directory.
  restore_glob=0
  case $- in
    *f*) ;;
    *)   restore_glob=1; set -f ;;
  esac

  # cmd   -- next token starts a command
  # flags -- inside git's global flags, still looking for the subcommand
  # skip  -- this token is the value of the flag before it
  # args  -- inside some other command's arguments; nothing here is a commit
  state=cmd
  result=1
  for tok in $normalized; do
    if [ "$state" = skip ]; then state=flags; continue; fi
    if [ "$tok" = ";" ]; then state=cmd; continue; fi
    case "$state" in
      cmd)
        case "$tok" in
          *=*)                         ;;  # env assignment prefix; still at a command position
          git|git.exe|*/git|*/git.exe) state=flags ;;
          *)                           state=args ;;
        esac
        ;;
      flags)
        case "$tok" in
          # Global flags that consume the NEXT token as their value. Missing one
          # here means its value gets read as the subcommand, and the gate stops
          # firing for every command that uses it.
          -C|-c|--git-dir|--work-tree|--namespace) state=skip ;;
          commit)                                  result=0; break ;;
          -*)                                      ;;  # self-contained flag, incl. --git-dir=x
          *)                                       state=args ;;  # some other subcommand
        esac
        ;;
    esac
  done

  [ "$restore_glob" = 1 ] && set +f
  return "$result"
}

# ---------------------------------------------------------------------------
# Two views of the command, and they are not interchangeable:
#
#   normalize_command  collapses every quoted region to one opaque token. Right
#                      for "what sits at a command position", wrong for anything
#                      that needs a VALUE back out.
#   tokenize_command   preserves the contents of quoted regions, so `cd "/a b"`
#                      yields the path. Correspondingly more expensive: it is a
#                      character walk, so callers reach for it only when the
#                      cheap view says there is something to resolve.
# ---------------------------------------------------------------------------

# Quoted regions collapse to ONE opaque token rather than being deleted.
# Deleting them looks equivalent and is not: `git -C "/path with space" commit`
# would become `git -C  commit`, and the -C would then swallow the word
# `commit` as its own value -- a commit that reports itself as not a commit.
#
# Newlines become separators, so the second line of a two-line script is still
# a command position.
normalize_command() {
  printf '%s' "$HOOK_COMMAND" \
    | sed -e "s/'[^']*'/__CCQ__/g" -e 's/"[^"]*"/__CCQ__/g' \
    | tr '\n' ';' \
    | sed -e 's/[;&|()][;&|()]*/ ; /g'
}

# Unquoted word splitting over a token stream would otherwise glob-expand a token
# like `*` against the working directory. Paired: every push needs a pop.
noglob_push() {
  case $- in
    *f*) NOGLOB_RESTORE=0 ;;
    *)   NOGLOB_RESTORE=1; set -f ;;
  esac
}
noglob_pop() {
  [ "${NOGLOB_RESTORE:-0}" = 1 ] && set +f
  return 0
}

# Words that leave the NEXT word at a command position: shell keywords and
# grouping (`{ git commit; }`, `if git commit; then`, `! git commit`) and the
# plain precommand wrappers (`time git commit`, `env X=1 git commit`).
#
# Every walker below that tracks a command position asks this one question, so
# they cannot disagree about it. They did: none of them knew these words, so
# `{ git commit -m x; }` was not a commit at all to is_git_commit and the gate
# silently never fired -- the unsafe direction. Teaching only one walker is no
# better: the gate then fires, and judges the target repo or the opt-out from a
# different reading of the same command.
#
# A wrapper's own flags (`env -i`, `time -p`, `exec -a name`) are not modelled.
# The walk reads them as arguments and misses that commit rather than guess.
is_command_prefix() {
  case "$1" in
    '{'|'}'|'!'|if|then|else|elif|do|while|until|time|command|exec|nohup|env) return 0 ;;
  esac
  return 1
}

# The same walk for any subcommand: `is_git_subcommand push`. One walker, so a
# fix to git's global-flag handling reaches every hook that asks.
is_git_subcommand() {
  local want=$1 normalized tok state result

  normalized=$(normalize_command)
  noglob_push

  # cmd   -- next token starts a command
  # flags -- inside git's global flags, still looking for the subcommand
  # skip  -- this token is the value of the flag before it
  # args  -- inside some other command's arguments; nothing here is a commit
  state=cmd
  result=1
  for tok in $normalized; do
    if [ "$state" = skip ]; then state=flags; continue; fi
    if [ "$tok" = ";" ]; then state=cmd; continue; fi
    case "$state" in
      cmd)
        is_command_prefix "$tok" && continue
        case "$tok" in
          *=*)                         ;;  # env assignment prefix; still at a command position
          git|git.exe|*/git|*/git.exe) state=flags ;;
          *)                           state=args ;;
        esac
        ;;
      flags)
        case "$tok" in
          # Global flags that consume the NEXT token as their value. Missing one
          # here means its value gets read as the subcommand, and the gate stops
          # firing for every command that uses it.
          -C|-c|--git-dir|--work-tree|--namespace) state=skip ;;
          "$want")                                 result=0; break ;;
          -*)                                      ;;  # self-contained flag, incl. --git-dir=x
          *)                                       state=args ;;  # some other subcommand
        esac
        ;;
    esac
  done

  noglob_pop
  return "$result"
}

# Splits HOOK_COMMAND into the TOKENS array, keeping the CONTENTS of quoted
# regions (that is the whole difference from normalize_command). Shell separators
# become a bare ";" token; a closing paren stays ")" so the walker can discard a
# subshell's `cd`, which does not outlive it.
tokenize_command() {
  local cmd=$1 i=0 n ch q='' tok='' started=0
  TOKENS=()
  n=${#cmd}
  while [ "$i" -lt "$n" ]; do
    ch=${cmd:i:1}
    i=$((i + 1))
    if [ -n "$q" ]; then
      if [ "$ch" = "$q" ]; then
        q=''
      elif [ "$q" = '"' ] && [ "$ch" = '\' ] && [ "$i" -lt "$n" ]; then
        tok+=${cmd:i:1}; i=$((i + 1))
      else
        tok+=$ch
      fi
      continue
    fi
    case "$ch" in
      "'"|'"') q=$ch; started=1 ;;
      '\')     if [ "$i" -lt "$n" ]; then tok+=${cmd:i:1}; i=$((i + 1)); fi ;;
      ' '|$'\t')
        if [ -n "$tok" ] || [ "$started" = 1 ]; then TOKENS+=("$tok"); tok=''; started=0; fi ;;
      ';'|'&'|'|'|'('|')'|$'\n')
        if [ -n "$tok" ] || [ "$started" = 1 ]; then TOKENS+=("$tok"); tok=''; started=0; fi
        if [ "$ch" = ')' ]; then TOKENS+=(")"); else TOKENS+=(";"); fi ;;
      *) tok+=$ch ;;
    esac
  done
  if [ -n "$tok" ] || [ "$started" = 1 ]; then TOKENS+=("$tok"); fi
}

# Resolves $2 as a directory relative to $1. Fails on anything whose value this
# process cannot know -- a substitution, `cd -`, a bare `cd` -- because a guessed
# directory is worse than no directory: see commit_target_dir.
resolve_dir() {
  case "$2" in
    ''|-|--) return 1 ;;
    *'$'*|*'`'*) return 1 ;;
    /*)  printf '%s' "$2" ;;
    '~') printf '%s' "$HOME" ;;
    '~/'*) printf '%s%s' "$HOME" "${2#\~}" ;;
    *)   printf '%s/%s' "$1" "$2" ;;
  esac
}

# The directory the `git commit` in HOOK_COMMAND will actually run in, honouring
# a leading `cd`/`pushd` and git's own -C. Fails when that cannot be determined.
#
# Deriving this from the command rather than from $PWD is the point: the hook
# process runs in the SESSION's repo, which is not necessarily the repo the
# command targets. `cd /other/repo && git commit` and `git -C /other/repo commit`
# used to be judged against the session's CHANGELOG.md instead -- blocking a
# clean cross-repo commit and waving through a dirty one.
# (LL-G kb/claude-code/hook-cwd-is-not-the-commit-target-repo.md)
#
# Failure means ALLOW, not block. A gate that refuses commits in repos it cannot
# even see gets bypassed wholesale, and it takes the changelog check with it.

# The same resolution for any subcommand: `git_target_dir push`.
git_target_dir() {
  local want=$1 base=${HOOK_CWD:-$PWD} tok state curdir gitdir found

  # Cheap view first: with no directory-changing token anywhere outside quotes,
  # the command runs where the hook does and there is nothing to walk.
  noglob_push
  found=0
  for tok in $(normalize_command); do
    case "$tok" in
      cd|pushd|-C|--git-dir|--git-dir=*|--work-tree|--work-tree=*) found=1; break ;;
    esac
  done
  noglob_pop
  if [ "$found" = 0 ]; then printf '%s' "$base"; return 0; fi

  tokenize_command "$HOOK_COMMAND"
  state=cmd
  curdir=$base
  gitdir=$base
  for tok in "${TOKENS[@]}"; do
    # A separator where a value was expected means the command is not what it
    # looked like; do not guess at the target.
    case "$state" in
      cdarg|cflag) case "$tok" in ';'|')') return 1 ;; esac ;;
      *)
        case "$tok" in
          ';') state=cmd; continue ;;
          ')') state=cmd; curdir=$base; continue ;;  # a subshell's cd did not survive it
        esac ;;
    esac
    case "$state" in
      cmd)
        is_command_prefix "$tok" && continue
        case "$tok" in
          cd|pushd)                    state=cdarg ;;
          git|git.exe|*/git|*/git.exe) state=flags; gitdir=$curdir ;;
          *=*)                         ;;  # env assignment prefix; still a command position
          *)                           state=args ;;
        esac ;;
      cdarg)
        curdir=$(resolve_dir "$curdir" "$tok") || return 1
        state=args ;;
      flags)
        case "$tok" in
          -C)                             state=cflag ;;
          -c|--namespace)                 state=skip ;;
          # An explicit git dir retargets the repo without moving the cwd, and
          # the tree it names may not be a working tree at all. Out of scope.
          --git-dir*|--work-tree*)        return 1 ;;
          "$want")                        printf '%s' "$gitdir"; return 0 ;;
          -*)                             ;;
          *)                              state=args ;;
        esac ;;
      cflag)
        gitdir=$(resolve_dir "$gitdir" "$tok") || return 1
        state=flags ;;
      skip)
        state=flags ;;
    esac
  done
  return 1
}

# Anchors every subsequent git call to the root of the repo the COMMAND targets,
# so a commit issued from a subdirectory -- or from another repo entirely -- is
# judged against the tree it will actually land in. $1 names the subcommand
# whose target counts (default: commit).
anchor_to_repo_root() {
  local target root
  target=$(git_target_dir "${1:-commit}") || return 1
  [ -d "$target" ] || return 1
  cd "$target" || return 1
  root=$(git rev-parse --show-toplevel 2>/dev/null) || return 1
  cd "$root" || return 1
}

# Commits that legitimately carry no changelog entry.
is_changelog_exempt() {
  # Merge commits: the merged branches carry their own entries.
  if git rev-parse -q --verify MERGE_HEAD >/dev/null 2>&1; then
    return 0
  fi
  # Explicit opt-out for reverts, hotfixes, genuinely trivial commits.
  if [ "${SKIP_CHANGELOG:-}" = "1" ]; then
    return 0
  fi
  return 1
}

# True when CHANGELOG.md is already staged, or when this very command stages it.
# The hook fires BEFORE the command runs, so a compound
# "git add CHANGELOG.md && git commit ..." has not staged it yet.
changelog_is_handled() {
  if printf '%s' "$HOOK_COMMAND" | grep -qE 'git add [^&|;]*CHANGELOG\.md'; then
    return 0
  fi
  git diff --cached --name-only 2>/dev/null | grep -q '^CHANGELOG\.md$'
}
