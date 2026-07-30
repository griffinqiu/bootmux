#compdef _bootmux bootmux

_bootmux() {
  local commands projects
  local -i command_index=2
  commands=(${(f)"$(bootmux commands zsh)"})
  projects=(${(f)"$(bootmux completions start)"})

  if [[ $words[2] == --backend ]]; then
    if (( CURRENT == 3 )); then
      _values 'backend' tmux herdr
      return
    fi
    command_index=4
  elif [[ $words[2] == --backend=* ]]; then
    command_index=3
  fi

  if (( CURRENT == command_index )); then
    _alternative \
      'commands:: _describe -t commands "bootmux subcommands" commands' \
      'projects:: _describe -t projects "bootmux projects" projects'
  elif (( CURRENT == command_index + 1 )); then
    case $words[command_index] in
      bindings)
        _values 'backend' tmux herdr
      ;;
      copy|cp|c|debug|delete|rm|open|o|start|s|stop|edit|e)
        _arguments '*:projects:($projects)'
      ;;
    esac
  fi

  return
}


# Local Variables:
# mode: Shell-Script
# sh-indentation: 2
# indent-tabs-mode: nil
# sh-basic-offset: 2
# End:
# vim: ft=zsh sw=2 ts=2 et
