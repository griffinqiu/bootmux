class Bootmux < Formula
  desc "Run tmuxinator-style YAML projects in tmux, Herdr, or zellij"
  homepage "https://github.com/griffinqiu/bootmux"
  version "0.3.1"
  license "MIT"

  head do
    url "https://github.com/griffinqiu/bootmux.git", branch: "main"

    depends_on "rust" => :build
  end

  on_macos do
    on_arm do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.3.1/bootmux-aarch64-apple-darwin.tar.gz"
      sha256 "56b652a006fd28039f62e23ab751755dc70418771353b01a83926543032e7f8b"
    end
    on_intel do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.3.1/bootmux-x86_64-apple-darwin.tar.gz"
      sha256 "9ea19847214c14b1ef350a90d414d712dfdd7b6275235d95bbcc9f6d0a768d79"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.3.1/bootmux-aarch64-unknown-linux-musl.tar.gz"
      sha256 "158b718115a1cc5f8c0f335a4e7330c4a22fbe3db3036438f6bd57b98966da3a"
    end
    on_intel do
      url "https://github.com/griffinqiu/bootmux/releases/download/v0.3.1/bootmux-x86_64-unknown-linux-musl.tar.gz"
      sha256 "cd3a587e361ace88dc4950256e30f83deb757329d5544b46f9d7e026a2a752d9"
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
