class Bootmux < Formula
  desc "Run tmuxinator-style YAML projects in tmux, Herdr, or zellij"
  homepage "https://github.com/griffinqiu/bootmux"
  version "0.1.5"
  license "MIT"

  head do
    url "https://github.com/griffinqiu/bootmux.git", branch: "main"

    depends_on "rust" => :build
  end

  on_macos do
    on_arm do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.1.5/bootmux-aarch64-apple-darwin.tar.gz"
      sha256 "305bdc71c42cee0b44932e86702577794e53cbc4e84778614c378d3e2950e47f"
    end
    on_intel do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.1.5/bootmux-x86_64-apple-darwin.tar.gz"
      sha256 "611344e53d7f767fe42b42b6d02d0c271bd8d931cd663f3b17630990e952e95b"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.1.5/bootmux-aarch64-unknown-linux-musl.tar.gz"
      sha256 "758595a8c337af1014ec9f892c7f3dde994acdc40a939ec530d6480b2399acd6"
    end
    on_intel do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.1.5/bootmux-x86_64-unknown-linux-musl.tar.gz"
      sha256 "ddd4e318b3f0b6efafa5f018b6a6ae7d8ca7401c9d1e5a57466dd515882029f3"
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
