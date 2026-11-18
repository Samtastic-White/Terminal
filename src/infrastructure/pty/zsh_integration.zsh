if [[ $- == *i* && -n ${NATIVE_TERMINAL:-} && -z ${NATIVE_TERMINAL_ACTIVE:-} ]]; then
  export NATIVE_TERMINAL_ACTIVE=1

  typeset -ga precmd_functions preexec_functions

  # Engine mode: the GPUI editor owns the prompt region.
  unsetopt zle prompt_sp prompt_cr
  PS1='' RPROMPT='' PROMPT2='' PROMPT_EOL_MARK=''
  [[ -t 0 ]] && stty -echo 2>/dev/null

  _native_terminal_precmd() {
    local ret=$?
    [[ -t 0 ]] && stty -echo 2>/dev/null
    printf '\033]133;D;%s\033\\' "$ret"
    printf '\033]633;P;Cwd=%s\033\\' "$PWD"
    printf '\033]133;A\033\\'
    return $ret
  }

  _native_terminal_preexec() {
    [[ -t 0 ]] && stty echo 2>/dev/null
    printf '\033]133;B\033\\'
    printf '\033]133;C\033\\'
    printf '\033]633;E;%s\033\\' "$1"
  }

  precmd_functions=(_native_terminal_precmd ${precmd_functions:#_native_terminal_precmd})
  preexec_functions=(_native_terminal_preexec ${preexec_functions:#_native_terminal_preexec})
fi
