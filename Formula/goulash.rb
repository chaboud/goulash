# Homebrew formula, served straight from this repo:
#   brew tap chaboud/goulash https://github.com/chaboud/goulash
#   brew install goulash
# The release workflow rewrites url/sha256 on every tagged release.
class Goulash < Formula
  desc "Your shell, with a coach — LLM overlay for zsh and bash"
  homepage "https://goulash.dev"
  url "https://github.com/chaboud/goulash/archive/refs/tags/v0.4.0.tar.gz"
  sha256 "ef3c080915009d9f9444c9bf9cbdd40607e7e65c6e1b4052bba43e46b12d90c6"
  license any_of: ["MIT", "Apache-2.0"]
  head "https://github.com/chaboud/goulash.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "goulash", shell_output("#{bin}/goulash --version")
  end
end
