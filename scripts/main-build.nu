# The main build a pull request's measurements are compared with, shared by scripts/size.nu,
# scripts/latency.nu and scripts/search-quality.nu: CI uploads each measurement as an artifact from every build, on main and
# on pull requests, and a pull request is compared with the main build it merged onto.

# The artifact `artifact` of the main build to compare `commit` with: the CI run of the main
# commit a pull request's merge commit sits on (or, for any other commit, its merge base with
# `branch`), or else the latest main run that uploaded one. `read` gets the path of `file` in it
# and returns what to keep, before the download is removed. Nothing when no run has one.
export def find-main-artifact [
    repo: string
    commit: string
    branch: string
    artifact: string
    file: string
    read: closure
]: nothing -> any {
    let base = if $commit == "" { "" } else { base-of $repo $commit $branch }
    let at_base = if $base == "" { [] } else { main-runs $repo $branch $"&head_sha=($base)" }
    let candidates = $at_base ++ (main-runs $repo $branch "&per_page=10")
    let directory = mktemp --directory
    mut found: any = null
    for run in $candidates {
        let target = $directory | path join ($run.id | into string)
        let download = ^gh run download $run.id --repo $repo --name $artifact --dir $target | complete
        let path = $target | path join $file
        if $download.exit_code == 0 and ($path | path exists) {
            $found = {
                data: (do $read $path)
                id: $run.id
                commit: $run.head_sha
                url: $run.html_url
                merge_base: ($run.head_sha == $base)
                base: $base
            }
            break
        }
    }
    rm --recursive $directory
    $found
}

def base-of [repo: string, commit: string, branch: string]: nothing -> string {
    let parents = ^gh api $"repos/($repo)/commits/($commit)" --jq "[.parents[].sha]" | complete
    let parents = if $parents.exit_code == 0 {
        $parents.stdout | from json
    } else {
        []
    }
    if ($parents | length) == 2 {
        return $parents.0
    }
    let compared = ^gh api $"repos/($repo)/compare/($branch)...($commit)" --jq ".merge_base_commit.sha" | complete
    if $compared.exit_code == 0 {
        $compared.stdout | str trim
    } else {
        ""
    }
}

def main-runs [repo: string, branch: string, query: string]: nothing -> list<record> {
    let runs = ^gh api $"repos/($repo)/actions/workflows/ci.yml/runs?branch=($branch)&event=push($query)" --jq "[.workflow_runs[] | {id, head_sha, html_url}]" | complete
    if $runs.exit_code == 0 {
        $runs.stdout | from json
    } else {
        []
    }
}
