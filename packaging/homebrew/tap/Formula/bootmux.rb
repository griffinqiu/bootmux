class Bootmux < Formula
  desc "Run tmuxinator-style YAML projects in tmux, Herdr, or zellij"
  homepage "https://github.com/griffinqiu/bootmux"
  version "0.3.2"
  license "MIT"

  head do
    url "https://github.com/griffinqiu/bootmux.git", branch: "main"

    depends_on "rust" => :build
  end

  on_macos do
    on_arm do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.3.2/bootmux-aarch64-apple-darwin.tar.gz"
      sha256 "75ed6cbf70234622d820fbbbba971a0d50904c98c1434ebff71903f508172b41"
    end
    on_intel do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.3.2/bootmux-x86_64-apple-darwin.tar.gz"
      sha256 "5d9a98d7c97920e1e999e31d4e40c0a88d19634b860c564c53c995540e48c2ca"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.3.2/bootmux-aarch64-unknown-linux-musl.tar.gz"
      sha256 "03ffcc609ba4ba4165d2bc3ea304da70971066311daec0fff640781992b37b8d"
    end
    on_intel do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.3.2/bootmux-x86_64-unknown-linux-musl.tar.gz"
      sha256 "27db2d322e227b8de6b9c8689889ff53a76842abf5c7e6849059f04dee592112"
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
