# Goulash shell integration for zsh.
#
# Add to ~/.zshrc:
#   [[ -n "$GOULASH" ]] && source /path/to/goulash/shell/goulash.zsh
#
# Emits private OSC 7770 marks (stripped by goulash, ignored by bare
# terminals) so goulash learns prompt/command boundaries, command text,
# exit codes, and cwd.

(( ${+GOULASH} )) || return 0

__goulash_osc() { printf '\033]7770;%s\007' "$1"; }

__goulash_b64() { printf '%s' "$1" | command base64 | command tr -d '\n'; }

__goulash_precmd() {
  local code=$?
  __goulash_osc "D;${code}"
  __goulash_osc "P;$(__goulash_b64 "$PWD")"
  __goulash_osc "A"
}

__goulash_preexec() {
  __goulash_osc "B;$(__goulash_b64 "$1")"
}

autoload -Uz add-zsh-hook
add-zsh-hook precmd __goulash_precmd
add-zsh-hook preexec __goulash_preexec

# `#` aside: intercepted at accept-line, shipped to goulash over the OSC
# channel — never executed, kept in history. `\#` escapes to a real
# comment line for the shell.
__goulash_accept_line() {
  case "$BUFFER" in
    '\#'*)
      BUFFER="${BUFFER#\\}"
      ;;
    '#'*)
      __goulash_osc "Q;$(__goulash_b64 "$BUFFER")"
      print -s -- "$BUFFER"
      BUFFER=""
      zle reset-prompt
      return 0
      ;;
  esac
  zle .accept-line
}
zle -N accept-line __goulash_accept_line

# Down arrow: ordinary movement always wins — multiline buffers and
# history-forward behave exactly as before. Only past the end of history
# does Down ask goulash to pull the top suggestion into the line.
__goulash_down_or_suggest() {
  if [[ "$RBUFFER" == *$'\n'* ]] || (( HISTNO < HISTCMD )); then
    zle down-line-or-history
    return
  fi
  __goulash_osc "S;$(__goulash_b64 "$BUFFER")"
}
zle -N __goulash_down_or_suggest
bindkey '^[[B' __goulash_down_or_suggest
bindkey '^[OB' __goulash_down_or_suggest
