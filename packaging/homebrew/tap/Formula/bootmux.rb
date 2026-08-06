class Bootmux < Formula
  desc "Run tmuxinator-style YAML projects in tmux, Herdr, or zellij"
  homepage "https://github.com/griffinqiu/bootmux"
  url "https://github.com/griffinqiu/bootmux/archive/refs/tags/v0.1.3.tar.gz"
  sha256 "a96fd1d8446ae819a58e333bd1b29e90b85d11764be3cfa23cc7837d718239ab"
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
