# Whether a scheduled nightly has anything new to publish. The update feed on the `nightly`
# release is what installed copies are offered, and package-app.nu writes the commit it was
# built from into its version (`0.1.0-nightly.417+912be04`), so the feed, not the tag or the
# release's target, says which commit users already have.

# The commit `feed` (update.json's text) was built from, or why it can't be read.
export def published-commit [feed: string]: nothing -> record {
    let parsed = try {
        $feed | from json
    } catch {
        null
    }
    let version = if ($parsed | describe | str starts-with record) { $parsed.version? } else { null }
    if ($version | describe) != string {
        return {commit: null, problem: "update.json has no version"}
    }
    let commit = $version | parse --regex '\+(?<commit>[0-9a-f]+)$' | get commit?.0?
    if $commit == null or ($commit | str length) < 7 {
        return {
            commit: null
            problem: $"update.json's version ($version) names no commit"
        }
    }
    {commit: $commit, problem: null}
}

# Whether to build `head`, given the published nightly's commit (`null` for none) and any
# problem reading the feed. The feed may hold an abbreviated commit, so it matches by prefix.
export def decide [head: string, published: record]: nothing -> record {
    if $published.problem? != null {
        return {
            build: true
            level: warning
            message: $"($published.problem); building ($head) anyway"
        }
    }
    if $published.commit == null {
        return {
            build: true
            level: notice
            message: $"No nightly is published yet; building ($head)"
        }
    }
    if ($head | str starts-with $published.commit) {
        return {
            build: false
            level: notice
            message: $"Nightly is already up to date with main at ($head)"
        }
    }
    {
        build: true
        level: notice
        message: $"main moved from ($published.commit) to ($head); building it"
    }
}

# The published nightly's commit, read from the update feed on the `nightly` release.
def fetch-published [repo: string]: nothing -> record {
    let release = ^gh api $"repos/($repo)/releases/tags/nightly" | complete
    if $release.exit_code != 0 {
        if ($release.stderr | str contains "HTTP 404") {
            return {commit: null, problem: null}
        }
        return {
            commit: null
            problem: $"Couldn't read the nightly release: ($release.stderr | str trim)"
        }
    }
    let feed = ^gh release download nightly --repo $repo --pattern update.json --output - | complete
    if $feed.exit_code != 0 {
        return {
            commit: null
            problem: $"Couldn't download update.json: ($feed.stderr | str trim)"
        }
    }
    published-commit $feed.stdout
}

# Decide whether `head` needs a new nightly, as a GitHub Actions annotation and `build=` output.
def main [repo: string, head: string] {
    let decision = decide $head (fetch-published $repo)
    print $"::($decision.level) title=Nightly::($decision.message)"
    if "GITHUB_OUTPUT" in $env {
        $"build=($decision.build)\n" | save --append $env.GITHUB_OUTPUT
    }
}
