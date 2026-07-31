# Goulash shell integration for zsh.
#
# Add to ~/.zshrc:
#   [[ -n "$GOULASH" ]] && source /path/to/goulash/shell/goulash.zsh
#
# Emits private OSC 7770 marks (stripped by goulash, ignored by bare
# terminals) so goulash learns prompt/command boundaries, command text,
# exit codes, and cwd.
#
# CONTRACT: this file changes nothing about zsh except the Up/Down
# arrows and the async interception of `#` lines. It sets no options,
# and every widget and key it touches is captured first and delegated
# to, so plugins that were already bound keep working. See
# wiki/architecture/shell-integration.md ("Adapter fidelity audit").

(( ${+GOULASH} )) || return 0

# Re-sourcing would capture OUR OWN widgets as the delegates and
# recurse forever. Load exactly once.
(( ${+__goulash_loaded} )) && return 0
typeset -g __goulash_loaded=1

__goulash_osc() { printf '\033]7770;%s\007' "$1"; }

__goulash_b64() { printf '%s' "$1" | command base64 | command tr -d '\n'; }

# --- hooks -------------------------------------------------------------
# $? has to be read before anything else runs at the prompt. add-zsh-hook
# appends, and the adapter loads after the user's .zshrc, so an appended
# hook sits behind every hook they registered and reads whatever the last
# one happened to leave. Capture in a hook of its own, moved to the front
# — and hand the original status straight back so the user's hooks still
# see exactly what they see without us.
typeset -g __goulash_code=0
__goulash_status() { return $1 }
__goulash_capture() {
  __goulash_code=$?
  return $__goulash_code
}

typeset -g __goulash_hist=""
__goulash_precmd() {
  # An aside is pushed into history HERE, not in the widget: zsh owns
  # the current history slot for as long as ZLE is editing it, so a
  # `print -s` from inside accept-line is discarded when the (blanked)
  # line is accepted. By the next prompt the slot is settled.
  if [[ -n "$__goulash_hist" ]]; then
    print -s -- "$__goulash_hist"
    __goulash_hist=""
  fi
  __goulash_osc "D;${__goulash_code}"
  # Only when it moved: this is the one hook on the steady-state path
  # and $(...) plus base64 is two forks per prompt.
  if [[ "$PWD" != "$__goulash_cwd" ]]; then
    typeset -g __goulash_cwd="$PWD"
    __goulash_osc "P;$(__goulash_b64 "$PWD")"
  fi
  __goulash_osc "A"
  __goulash_slot_buf=""
  __goulash_expect=0
}

__goulash_preexec() {
  __goulash_osc "B;$(__goulash_b64 "$1")"
}

autoload -Uz add-zsh-hook
add-zsh-hook precmd __goulash_capture
add-zsh-hook precmd __goulash_precmd
add-zsh-hook preexec __goulash_preexec
# add-zsh-hook only appends; the capture has to lead.
precmd_functions=(__goulash_capture ${precmd_functions:#__goulash_capture})

# The classic form — a bare `precmd` function — is called by zsh ahead of
# precmd_functions entirely, so being first in the array is not enough.
# Wrap it, restoring $? before it runs so it sees what it always saw.
if (( ${+functions[precmd]} )) && [[ ${functions[precmd]} != *__goulash_* ]]; then
  functions[__goulash_user_precmd]=${functions[precmd]}
  precmd() {
    __goulash_code=$?
    __goulash_status $__goulash_code
    __goulash_user_precmd "$@"
  }
fi

# --- widget capture ----------------------------------------------------
# Re-register whatever is bound today under a private name so we can call
# it. `$widgets[name]` is `builtin`, `user:fn`, or `completion:...`.
#
# These set globals rather than printing: `zle -N` inside a $(...) runs
# in a subshell, so the private alias would be registered in a process
# that promptly exits and the delegation would call a widget that does
# not exist — which reads, at the terminal, as a shell that echoes every
# line and runs none of them.
__goulash_capture_widget() {
  local name=$1 alias=$2 out=$3 cur=${widgets[$1]}
  if [[ "$cur" == user:* ]]; then
    zle -N "$alias" "${cur#user:}"
    typeset -g "$out=$alias"
  else
    typeset -g "$out=.$name"
  fi
}
__goulash_capture_widget accept-line __goulash_prev_accept __goulash_accept
__goulash_capture_widget bracketed-paste __goulash_prev_paste __goulash_paste

# Arrows are bound, not wrapped, so the question is what key sequence
# resolves to today. Anything already there gets called in our place on
# the paths where goulash has nothing to add.
__goulash_bound_widget() {
  local -a parts
  parts=( ${(z)"$(bindkey -- "$1" 2>/dev/null)"} )
  local w=${parts[2]}
  [[ -z "$w" || "$w" == undefined-key || "$w" == __goulash_* ]] && w=$2
  typeset -g "$3=$w"
}
__goulash_bound_widget $'\e[B' down-line-or-history __goulash_down
__goulash_bound_widget $'\e[A' up-line-or-history __goulash_up
__goulash_bound_widget $'\eOB' "$__goulash_down" __goulash_down_ss3
__goulash_bound_widget $'\eOA' "$__goulash_up" __goulash_up_ss3

# --- geometry repair ---------------------------------------------------
# goulash resizes the inner PTY when its own area changes height (a menu
# opening, chat taking focus). zsh gets a SIGWINCH and redraws, but ZLE's
# accounting for a line it has already drawn can end up describing the
# old geometry — and a WRAPPED line redrawn shorter then clears one row
# too few, leaving the tail of the previous line stranded on screen.
#
# Rather than tiptoe around ZLE's bookkeeping, make it re-derive: a
# reset-prompt on every WINCH costs nothing and repairs a real terminal
# drag too, which no amount of care on our side could have avoided.
#
# Unconditional on purpose. The documented idiom is `zle && zle
# reset-prompt`, but `zle` reports INACTIVE inside a WINCH trap even
# while ZLE is reading — measured — so the guard suppresses exactly the
# case that needs it. When ZLE really is idle the call is a harmless
# no-op returning 0.
if (( ${+functions[TRAPWINCH]} )) && [[ ${functions[TRAPWINCH]} != *__goulash_* ]]; then
  functions[__goulash_user_winch]=${functions[TRAPWINCH]}
fi
typeset -g __goulash_prev_winch_trap="$(trap -p WINCH 2>/dev/null)"
TRAPWINCH() {
  (( ${+functions[__goulash_user_winch]} )) && __goulash_user_winch "$@"
  [[ -n "$__goulash_prev_winch_trap" ]] && eval "${${__goulash_prev_winch_trap#trap -- }% WINCH}"
  zle reset-prompt 2>/dev/null
  return 0
}

# --- the `#` aside -----------------------------------------------------
# Intercepted at accept-line, shipped to goulash over the OSC channel,
# never executed, kept in history. `\#` escapes it back out.
#
# The buffer is BLANKED rather than left for the shell to treat as a
# comment. That is what removes the dependency on `setopt
# interactivecomments` — an option whose blast radius is every command
# the user types (`echo a # b` changes meaning) and which breaks Tab
# completion outright, because a commented line gives the completion
# system no current word to filter by or replace. History expansion runs
# before tokenisation, so a comment would not have protected `# what
# does !! do` anyway; never letting the parser see the line does.
__goulash_accept_line() {
  case "$BUFFER" in
    '\#'*)
      # Not for goulash. A real comment does nothing, so neither does
      # this — but it lands in history exactly as a comment would.
      BUFFER="${BUFFER#\\}"
      __goulash_hist="$BUFFER"
      BUFFER=""
      ;;
    '#'*)
      __goulash_osc "Q;$(__goulash_b64 "$BUFFER")"
      __goulash_hist="$BUFFER"
      # Leave what was typed on screen. Clearing BUFFER makes zle redraw
      # the line as empty, which ERASES the question — a session of asks
      # reads back as a column of bare prompts, and the transcript loses
      # the half the user wrote. bash keeps it for free (a `#` line is a
      # real comment there, so nothing rubs it out); zsh has to be told.
      #
      # `zle -I` hands the display back, so the typed line is final
      # before the buffer is cleared. Nothing else: a `print` here adds
      # a newline that accept-line then adds again, costing a blank row
      # and a repeated prompt on every ask.
      zle -I
      BUFFER=""
      ;;
  esac
  zle $__goulash_accept
}
zle -N accept-line __goulash_accept_line

# Slot-space tracking: goulash's pulls come back as bracketed pastes.
# Wrapping the paste widget records exactly what landed, so "is the
# line a goulash slot?" is a local buffer comparison that cannot drift
# — the load-bearing check for two-way slot scrolling.
typeset -g __goulash_slot_buf=""
typeset -g __goulash_expect=0
__goulash_bracketed_paste() {
  zle $__goulash_paste
  if (( __goulash_expect )); then
    __goulash_slot_buf="$BUFFER"
    __goulash_expect=0
  fi
}
zle -N bracketed-paste __goulash_bracketed_paste

# Down arrow: ordinary movement always wins — multiline buffers and
# history-forward behave exactly as before, and "as before" means
# whatever widget was bound here when we loaded. Only past the end of
# history does Down ask goulash to pull a suggestion into the line (and
# pulls again step deeper into the slot history).
__goulash_down_or_suggest() {
  local fallback=${1:-$__goulash_down}
  if [[ "$RBUFFER" == *$'\n'* ]] || (( HISTNO < HISTCMD )); then
    zle $fallback
    return
  fi
  __goulash_expect=1
  __goulash_osc "S;$(__goulash_b64 "$BUFFER")"
}
__goulash_down_csi() { __goulash_down_or_suggest $__goulash_down }
__goulash_down_ss3w() { __goulash_down_or_suggest $__goulash_down_ss3 }
zle -N __goulash_down_csi
zle -N __goulash_down_ss3w
bindkey '^[[B' __goulash_down_csi
bindkey '^[OB' __goulash_down_ss3w

# Up arrow: one continuous axis. On a goulash slot (untouched since the
# paste), Up slides back toward the neutral empty line; everywhere else
# — including after any edit — it is whatever Up already did.
__goulash_up_or_history() {
  local fallback=${1:-$__goulash_up}
  if [[ -n "$__goulash_slot_buf" && "$BUFFER" == "$__goulash_slot_buf" \
        && "$LBUFFER" != *$'\n'* ]]; then
    __goulash_expect=1
    __goulash_osc "U;$(__goulash_b64 "$BUFFER")"
    return
  fi
  __goulash_slot_buf=""
  zle $fallback
}
__goulash_up_csi() { __goulash_up_or_history $__goulash_up }
__goulash_up_ss3w() { __goulash_up_or_history $__goulash_up_ss3 }
zle -N __goulash_up_csi
zle -N __goulash_up_ss3w
bindkey '^[[A' __goulash_up_csi
bindkey '^[OA' __goulash_up_ss3w
