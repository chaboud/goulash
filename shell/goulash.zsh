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
