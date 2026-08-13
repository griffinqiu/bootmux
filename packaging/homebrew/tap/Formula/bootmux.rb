class Bootmux < Formula
  desc "Run tmuxinator-style YAML projects in tmux, Herdr, or zellij"
  homepage "https://github.com/griffinqiu/bootmux"
  version "0.2.0"
  license "MIT"

  head do
    url "https://github.com/griffinqiu/bootmux.git", branch: "main"

    depends_on "rust" => :build
  end

  on_macos do
    on_arm do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.2.0/bootmux-aarch64-apple-darwin.tar.gz"
      sha256 "3b0a4ad0f2a9226249367e90e463aba730eacf5e02a664a85e6359882b27fa24"
    end
    on_intel do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.2.0/bootmux-x86_64-apple-darwin.tar.gz"
      sha256 "ae8527ca329a878ae0398c6ed56b864d9ee9f0ef1de9ca5837e9f4727da03de5"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.2.0/bootmux-aarch64-unknown-linux-musl.tar.gz"
      sha256 "886c6f273cc3b1cc8dca75004fe8eb1ab6027c3c2b03976daac06829aab11d36"
    end
    on_intel do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.2.0/bootmux-x86_64-unknown-linux-musl.tar.gz"
      sha256 "55e4de82391e5a33c864b445e7e5199034af45981edc52aaaaaae9c9577a8374"
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
