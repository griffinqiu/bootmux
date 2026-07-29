function __fish_bootmux_using_command
    set cmd (commandline -opc)
    if [ (count $cmd) -gt 1 ]
        if [ $argv[1] = $cmd[2] ]
            return 0
        end
    end
    return 1
end

complete --no-files --command bootmux --condition __fish_use_subcommand --exclusive --argument "(bootmux commands)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command start' --argument "(bootmux completions start)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command open' --argument "(bootmux completions open)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command stop' --argument "(bootmux completions stop)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command edit' --argument "(bootmux completions open)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command copy' --argument "(bootmux completions copy)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command delete' --argument "(bootmux completions delete)"
complete --no-files --command bootmux --condition '__fish_bootmux_using_command debug' --argument "(bootmux completions start)"
