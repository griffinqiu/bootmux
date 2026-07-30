class Bootmux < Formula
  desc "Manage tmux sessions and Herdr workspaces from tmuxinator-compatible YAML"
  homepage "https://github.com/griffinqiu/bootmux"
  url "https://github.com/griffinqiu/bootmux/archive/refs/tags/v0.1.1.tar.gz"
  sha256 "7f91ed3348e3bc3bc78a2afcbc9e7102177948b8b85c48aa770ce5303576b714"
  license "MIT"
  head "https://github.com/griffinqiu/bootmux.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args

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
    version_output = shell_output("#{bin}/bootmux --version")
    if build.stable?
      assert_match "bootmux #{version}", version_output
    else
      assert_match(/\Abootmux \d+\.\d+\.\d+/, version_output)
    end
  end
end
