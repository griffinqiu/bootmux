class Bootmux < Formula
  desc "Run tmuxinator-style YAML projects in tmux, Herdr, or zellij"
  homepage "https://github.com/griffinqiu/bootmux"
  version "0.3.0"
  license "MIT"

  head do
    url "https://github.com/griffinqiu/bootmux.git", branch: "main"

    depends_on "rust" => :build
  end

  on_macos do
    on_arm do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.3.0/bootmux-aarch64-apple-darwin.tar.gz"
      sha256 "7539f8b41ed9eaec50fb9df12c260c2db757aa842310d5ead34f005e6b7d5246"
    end
    on_intel do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.3.0/bootmux-x86_64-apple-darwin.tar.gz"
      sha256 "0e13ed752ce95670a623131fdd223f6d26ebdd8dc1376391e163febb4c10f289"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.3.0/bootmux-aarch64-unknown-linux-musl.tar.gz"
      sha256 "0274bcffa0248c696aa399c7cbc8f6d169bf83eb32f2f46bcd24850d7d0eceed"
    end
    on_intel do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.3.0/bootmux-x86_64-unknown-linux-musl.tar.gz"
      sha256 "3d99206e09d1efa572cd52c1d0b5cc1314dcd14ba1e334eee204a540e75c11c6"
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
