class Bootmux < Formula
  desc "Run tmuxinator-style YAML projects in tmux, Herdr, or zellij"
  homepage "https://github.com/griffinqiu/bootmux"
  version "0.1.4"
  license "MIT"

  head do
    url "https://github.com/griffinqiu/bootmux.git", branch: "main"

    depends_on "rust" => :build
  end

  on_macos do
    on_arm do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.1.4/bootmux-aarch64-apple-darwin.tar.gz"
      sha256 "1db86768373f8ca30013bf5318c64e076935a3a0eb9b1158aaa27909975ca798"
    end
    on_intel do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.1.4/bootmux-x86_64-apple-darwin.tar.gz"
      sha256 "79a5e9a9871e76126bcc8090851b88fb7c3b46cd8f5ab8ae6d50557d777be0f6"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.1.4/bootmux-aarch64-unknown-linux-musl.tar.gz"
      sha256 "254805d67fefa94bf00d90263f9fcef8d58fabdc96a3dad542ac498835ec3616"
    end
    on_intel do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.1.4/bootmux-x86_64-unknown-linux-musl.tar.gz"
      sha256 "589dbab4fef8491658456032a76bea834e93fafe9668f16b1b822eb6531a2ba7"
    end
  end

  def install
    if build.head?
      system "cargo", "install", *std_cargo_args
    else
      bin.install "bootmux"
    end

    bash_completion.install "completion/bootmux.bash" => "bootmux"
    zsh_completion.install "completion/bootmux.zsh" => "_bootmux"
    fish_completion.install "completion/bootmux.fish"
  end

  test do
    config = testpath/"brew-test.yml"
    config.write <<~YAML
      name: homebrew-test
      root: "#{testpath}"
      attach: false
      windows:
        - shell: echo homebrew
    YAML

    output = shell_output(
      "#{bin}/bootmux --backend herdr debug --project-config #{config}",
    )
    assert_match "backend: herdr", output
    assert_match "project: homebrew-test", output
    assert_match "pane[0] commands=1", output

    zellij_output = shell_output(
      "#{bin}/bootmux --backend zellij debug --project-config #{config}",
    )
    assert_match "backend: zellij", zellij_output
    assert_match "session: homebrew-test", zellij_output
    assert_match "tab name=\"shell\"", zellij_output

    version_output = shell_output("#{bin}/bootmux --version")
    if build.stable?
      assert_match "bootmux #{version}", version_output
    else
      assert_match(/\Abootmux \d+\.\d+\.\d+/, version_output)
    end
  end
end
