#compdef _bootmux bootmux

_bootmux() {
  local commands projects
  commands=(${(f)"$(bootmux commands zsh)"})
  projects=(${(f)"$(bootmux completions start)"})

  if (( CURRENT == 2 )); then
    _alternative \
      'commands:: _describe -t commands "bootmux subcommands" commands' \
      'projects:: _describe -t projects "bootmux projects" projects'
  elif (( CURRENT == 3)); then
    case $words[2] in
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
