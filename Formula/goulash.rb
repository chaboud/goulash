# Homebrew formula, served straight from this repo:
#   brew tap chaboud/goulash https://github.com/chaboud/goulash
#   brew install goulash
# The release workflow rewrites url/sha256 on every tagged release.
class Goulash < Formula
  desc "Your shell, with a coach — LLM overlay for zsh and bash"
  homepage "https://goulash.dev"
  url "https://github.com/chaboud/goulash/archive/refs/tags/v0.3.0.tar.gz"
  sha256 "f5f59da7680f358f8a617f0248216177e5a8ab753d574d41d6cd483b4e86a12e"
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
