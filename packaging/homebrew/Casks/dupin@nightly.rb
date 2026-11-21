cask "dupin@nightly" do
  version :latest
  sha256 :no_check

  url "https://github.com/x-ryoppippi/dupin/releases/download/nightly/Dupin.zip"
  name "Dupin Nightly"
  desc "Lightweight resident launcher"
  homepage "https://dupin.app/"

  auto_updates true
  conflicts_with cask: "dupin"
  depends_on arch: :arm64
  depends_on macos: ">= :sonoma"

  app "Dupin.app"

  zap trash: [
    "~/.config/dupin",
    "~/Library/Application Support/Dupin",
    "~/Library/Logs/Dupin",
  ]
end
