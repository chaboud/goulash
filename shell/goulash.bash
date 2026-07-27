# Goulash shell integration for bash.
#
# Add to ~/.bashrc:
#   [[ -n "$GOULASH" ]] && source /path/to/goulash/shell/goulash.bash
#
# Emits private OSC 7770 marks (stripped by goulash, ignored by bare
# terminals). preexec is approximated with a DEBUG trap in the style of
# bash-preexec: only the first simple command after a prompt counts.
#
# CONTRACT: this file changes nothing about bash except the async
# interception of `#` lines. It sets no options and no key bindings,
# and it chains onto any DEBUG trap and PROMPT_COMMAND already there
# rather than replacing them. See
# wiki/architecture/shell-integration.md ("Adapter fidelity audit").

[[ -n "$GOULASH" ]] || return 0

# Re-sourcing would chain us onto ourselves. Load exactly once.
[[ -n "$__goulash_loaded" ]] && return 0
__goulash_loaded=1

__goulash_osc() { printf '\033]7770;%s\007' "$1"; }

__goulash_b64() { printf '%s' "$1" | command base64 | command tr -d '\n'; }

__goulash_in_prompt=1
__goulash_cwd=""
__goulash_histnum=""

# Any DEBUG trap already installed (bash-preexec, a profiler, direnv)
# keeps running: chain onto it instead of replacing it.
#
# Reading it is the hard part. bash withholds the DEBUG trap from shell
# functions AND from sourced files unless `functrace` is on, so
# `trap -p DEBUG` reports the default in both — from here it looks like
# nobody has one, and the plugin gets silently dropped. Verified.
# PROMPT_COMMAND is evaluated in the top-level context, which is the
# only place the real answer is visible, so the read happens there and
# the value is handed in as an argument. Setting a trap from inside a
# function is fine; only reading one is blocked.
__goulash_prev_debug=""
__goulash_armed=0
__goulash_arm() {
  (( __goulash_armed )) && return
  __goulash_armed=1
  local t=$1
  t=${t#trap -- }
  t=${t% DEBUG}
  if [[ ${t:0:1} == "'" && ${t: -1} == "'" ]]; then
    t=${t:1:${#t}-2}
    # Undo the '\'' escaping `trap -p` uses for embedded quotes.
    t=${t//\'\\\'\'/\'}
  fi
  [[ $t == *__goulash_preexec* ]] || __goulash_prev_debug=$t
  trap '__goulash_preexec' DEBUG
  # Stop paying a fork per prompt for a one-shot question — and make
  # sure a second capture can never read our own trap back.
  if (( __goulash_pc_array )); then
    local -a keep=()
    local e
    for e in "${PROMPT_COMMAND[@]}"; do
      [[ $e == "$__goulash_boot" ]] || keep[${#keep[@]}]=$e
    done
    PROMPT_COMMAND=("${keep[@]}")
  else
    PROMPT_COMMAND=${PROMPT_COMMAND#"$__goulash_boot; "}
  fi
  return 0
}

# Every exit from here returns 0 on purpose: under `shopt -s extdebug` a
# DEBUG trap that returns non-zero SKIPS the command it fired for, and
# `[[ ... ]] && return` hands back 1 on the branch that did nothing.
__goulash_preexec() {
  [[ -n "$__goulash_prev_debug" ]] && eval "$__goulash_prev_debug"
  [[ -n "$COMP_LINE" ]] && return 0
  case "$BASH_COMMAND" in
    __goulash_*) return 0 ;;
  esac
  (( __goulash_in_prompt )) || return 0
  local h num cmd
  h=$(HISTTIMEFORMAT='' builtin history 1) || return 0
  h="${h#"${h%%[![:space:]]*}"}"
  num="${h%%[![:digit:]]*}"
  # A DEBUG hit with the history number unmoved is not the user's next
  # command — it is some other prompt hook running. Trusting the
  # in-prompt flag alone meant one plugin function in PROMPT_COMMAND
  # consumed the flag, so the real command that followed was never
  # reported and goulash sat in `cmd` forever.
  [[ -n "$num" && "$num" == "$__goulash_histnum" ]] && return 0
  [[ -n "$num" ]] && __goulash_histnum="$num"
  __goulash_in_prompt=0
  cmd="${h#"$num"}"
  cmd="${cmd#"${cmd%%[![:space:]]*}"}"
  __goulash_osc "B;$(__goulash_b64 "$cmd")"
  return 0
}

# Arming is deliberately the LAST thing at a prompt, not the first: any
# hook that runs after it fires the DEBUG trap and would be mistaken for
# the user's command.
__goulash_ready() { __goulash_in_prompt=1; return 0; }

# The `#` aside, bash edition. bash has `interactive_comments` on by
# default, so a `#` line is a comment: it never executes, the DEBUG trap
# never fires, and there is nothing to intercept the way zsh's ZLE
# accept-line widget does. What bash *does* do is record it in history —
# so the aside is recovered at the next prompt by noticing that the
# history number advanced onto a line starting with `#`.
#
# One command substitution per prompt is the price, and it buys the
# whole `#` / `##` / `#/` / `#@` surface for bash, which had none of it.
__goulash_aside() {
  local h num cmd
  h=$(HISTTIMEFORMAT='' builtin history 1) || return
  h="${h#"${h%%[![:space:]]*}"}"
  num="${h%%[![:digit:]]*}"
  [[ -z "$num" ]] && return
  cmd="${h#"$num"}"
  cmd="${cmd#"${cmd%%[![:space:]]*}"}"
  [[ "$num" == "$__goulash_histnum" ]] && return
  __goulash_histnum="$num"
  case "$cmd" in
    '\#'*) ;;
    '#'*) __goulash_osc "Q;$(__goulash_b64 "$cmd")" ;;
  esac
}

__goulash_precmd() {
  local code=$?
  __goulash_osc "D;${code}"
  # Only when it moved: $(...) plus base64 is two forks, and this runs
  # on every prompt.
  if [[ "$PWD" != "$__goulash_cwd" ]]; then
    __goulash_cwd="$PWD"
    __goulash_osc "P;$(__goulash_b64 "$PWD")"
  fi
  __goulash_aside
  __goulash_osc "A"
  return $code
}

# Run first so $? is the command's, not some other hook's — and chain,
# never replace. bash 5.1 allows PROMPT_COMMAND to be an array.
__goulash_boot='__goulash_arm "$(trap -p DEBUG)"'
# `${PROMPT_COMMAND@a}` is the obvious test and is a hard parse error on
# bash 3.2 — which is still the /bin/bash on every mac. `declare -p`
# answers the same question everywhere, once, at load.
__goulash_pc_array=0
case "$(declare -p PROMPT_COMMAND 2>/dev/null)" in
  "declare -a"*) __goulash_pc_array=1 ;;
esac
if (( __goulash_pc_array )); then
  PROMPT_COMMAND=("$__goulash_boot" __goulash_precmd "${PROMPT_COMMAND[@]}" __goulash_ready)
else
  PROMPT_COMMAND="$__goulash_boot; __goulash_precmd\
${PROMPT_COMMAND:+; $PROMPT_COMMAND}; __goulash_ready"
fi
