function __fish_bootmux_using_command
    set tokens (commandline -opc)
    if test (count $tokens) -lt 2
        return 1
    end
    set skip_next 0
    for token in $tokens[2..-1]
        if test $skip_next -eq 1
            set skip_next 0
            continue
        end
        switch $token
            case --backend
                set skip_next 1
            case '--backend=*' '-*'
                continue
            case '*'
                test "$argv[1]" = "$token"
                return $status
        end
    end
    return 1
end

complete --no-files --command bootmux --condition __fish_use_subcommand --exclusive --argument "(bootmux commands)"
complete --no-files --command bootmux --long-option backend --require-parameter --arguments "tmux herdr zellij"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command start' --argument "(bootmux completions start)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command open' --argument "(bootmux completions open)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command stop' --argument "(bootmux completions stop)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command edit' --argument "(bootmux completions open)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command copy' --argument "(bootmux completions copy)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command delete' --argument "(bootmux completions delete)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command debug' --argument "(bootmux completions start)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command bindings' --argument "tmux herdr zellij"
