# Griffin Qiu's Homebrew tap

## bootmux

Install [bootmux](https://github.com/griffinqiu/bootmux):

```sh
brew install griffinqiu/tap/bootmux
```

Upgrade or uninstall:

```sh
brew upgrade griffinqiu/tap/bootmux
brew uninstall griffinqiu/tap/bootmux
```

The formula installs the prebuilt executable attached to the tagged release,
along with Bash, Zsh, and Fish completions. It declares no dependencies, so
installing it never downloads a compiler toolchain. Only
`brew install --HEAD griffinqiu/tap/bootmux` builds from source and needs
`rust`. bootmux requires tmux 2.6+, Herdr 0.7.5/protocol 17 or 19, or both
at runtime.

The Formula is synchronized by the stable release workflow documented in
the [bootmux release guide](https://github.com/griffinqiu/bootmux/blob/main/docs/releasing.md).
