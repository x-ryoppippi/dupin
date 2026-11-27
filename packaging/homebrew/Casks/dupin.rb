cask "dupin" do
  version "0.1.0"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"

  url "https://github.com/x-ryoppippi/dupin/releases/download/v#{version}/Dupin.zip"
  name "Dupin"
  desc "Lightweight resident launcher"
  homepage "https://dupin.app/"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates true
  conflicts_with cask: "dupin@nightly"
  depends_on arch: :arm64
  depends_on macos: ">= :sonoma"

  app "Dupin.app"

  zap trash: [
    "~/.config/dupin",
    "~/Library/Application Support/Dupin",
    "~/Library/Caches/Dupin",
    "~/Library/Logs/Dupin",
    "~/Library/Preferences/app.dupin.Dupin.plist",
  ]
end
