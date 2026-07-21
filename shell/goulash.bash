# Goulash shell integration for bash.
#
# Add to ~/.bashrc:
#   [[ -n "$GOULASH" ]] && source /path/to/goulash/shell/goulash.bash
#
# Emits private OSC 7770 marks (stripped by goulash, ignored by bare
# terminals). preexec is approximated with a DEBUG trap in the style of
# bash-preexec: only the first simple command after a prompt counts.

[[ -n "$GOULASH" ]] || return 0

__goulash_osc() { printf '\033]7770;%s\007' "$1"; }

__goulash_b64() { printf '%s' "$1" | command base64 | command tr -d '\n'; }

__goulash_in_prompt=1

__goulash_preexec() {
  [[ -n "$COMP_LINE" ]] && return
  case "$BASH_COMMAND" in
    __goulash_*) return ;;
  esac
  (( __goulash_in_prompt )) || return
  __goulash_in_prompt=0
  local cmd
  cmd=$(HISTTIMEFORMAT='' builtin history 1)
  cmd="${cmd#*[0-9]  }"
  __goulash_osc "B;$(__goulash_b64 "$cmd")"
}

__goulash_precmd() {
  local code=$?
  __goulash_osc "D;${code}"
  __goulash_osc "P;$(__goulash_b64 "$PWD")"
  __goulash_osc "A"
  __goulash_in_prompt=1
}

PROMPT_COMMAND="__goulash_precmd${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
trap '__goulash_preexec' DEBUG
