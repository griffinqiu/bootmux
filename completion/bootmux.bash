_bootmux() {
    COMPREPLY=()
    local word
    word="${COMP_WORDS[COMP_CWORD]}"
    local command_index=1
    case "${COMP_WORDS[1]}" in
        --backend) command_index=3 ;;
        --backend=*) command_index=2 ;;
    esac

    if [ "$COMP_CWORD" -gt 0 ] && [ "${COMP_WORDS[COMP_CWORD-1]}" = "--backend" ]; then
        COMPREPLY=( $(compgen -W "tmux herdr" -- "$word") )
    elif [ "$COMP_CWORD" -eq "$command_index" ]; then
        local commands="$(compgen -W "$(bootmux commands)" -- "$word")"
        local projects="$(compgen -W "$(bootmux completions start)" -- "$word")"

        COMPREPLY=( $commands $projects )
    elif [ "$COMP_CWORD" -eq $((command_index + 1)) ] && [ "${COMP_WORDS[command_index]}" = "bindings" ]; then
        COMPREPLY=( $(compgen -W "tmux herdr" -- "$word") )
    elif [ "$COMP_CWORD" -eq $((command_index + 1)) ]; then
        local completions
        completions=$(bootmux completions "${COMP_WORDS[command_index]}")
        COMPREPLY=( $(compgen -W "$completions" -- "$word") )
    fi
}

complete -F _bootmux bootmux
