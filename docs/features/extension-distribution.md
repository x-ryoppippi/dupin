# Distributing extensions

An extension is shared as a Git repository. There is no store, hosted index or publishing account: push the package, tag its versions, and add the `dupin-extension` topic so `dupin ext search` lists it. People install it with `dupin ext install`, which never runs anything from the package, so commit the entry module Dupin runs.

```sh
dupin ext build .                          # only for a package with npm dependencies
git tag v1.0.0 && git push origin main v1.0.0
dupin ext install github:someone/my-extension   # takes the newest version tag
```

| Step      | Rule                                                                                                                                                                |
| --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Entry     | `dupin.main` is committed: JavaScript or erasable TypeScript as written, or the bundle a build made. The `--bundle` scaffold's workflow commits the bundle for you. |
| Versions  | Tags such as `v1.2.3` or `1.2.3`. Without one, an install takes the default branch.                                                                                 |
| Discovery | The `dupin-extension` GitHub topic; [`dupin ext new`](./extension-testing.md#creating-a-package) also writes `"keywords": ["dupin-extension"]`.                     |
| Size      | At most 1,024 files, 4 MiB per file and 16 MiB in all; see [What an install checks](#what-an-install-checks).                                                       |

The installer reads the package as the [extension platform design](../design/extension-platform.md#packaging-and-distribution) describes it.

## Building a bundle

`dupin ext build <dir> [--sourcemap]` bundles a package with `source` into a single ES module at `dupin.main`, with its npm dependencies, by [Rolldown](https://rolldown.rs). A package without `source` runs as written, JavaScript or TypeScript, and needs no build; `build` only checks it.

Start a package that uses npm packages with the `--bundle` scaffold. It pins Rolldown, writes the configuration, and adds a workflow that rebuilds `dist/main.js` and commits it when a push to `main` changes its sources ([The `--bundle` scaffold](./extension-testing.md#creating-a-package)). [`examples/extensions/change-case`](../../examples/extensions/change-case) is one, with a single dependency.

```sh
dupin ext new my-extension --bundle
cd my-extension
bun add some-package    # then import it from src/main.ts
dupin ext build .
```

To convert an existing package, add `"source": "src/main.ts"` to `package.json`, point `dupin.main` at a JavaScript file such as `dist/main.js`, add Rolldown with `bun add -d rolldown` (or `pnpm add -D rolldown`), and run `dupin ext build .`. It needs no Rolldown configuration of its own: without one it uses the scaffold's. To rebuild in CI, copy the scaffold's `rolldown.config.ts` and `.github/workflows/build.yml` too, since the workflow runs Rolldown without Dupin. That workflow installs from `bun.lock`, watches `src/`, and uploads and commits `dist/main.js`, so use Bun, or change those paths to match your package.

Dupin uses the package's own `node_modules/.bin/rolldown`, or else `rolldown` on `PATH`. It never downloads it; with neither, the build fails and says how to add it. Rolldown's CLI runs on Node.js.

<details>
<summary>Configuration</summary>

The build uses the package's `rolldown.config.*`, which the `--bundle` scaffold writes, and a package without one is bundled with the same configuration built into Dupin. A `rolldown.config.js` or `.mjs` is refused, because the launcher loads every `.js` and `.mjs` file in the package as extension code; use `.ts`, `.mts`, `.cts` or `.cjs`. `dupin ext build` sets only the output file and, with `--sourcemap`, a linked `<main>.map`.

The configuration produces one ES module for `platform: "neutral"`, with dynamic imports inlined and no other chunks, keeps `dupin` as the only external import, and reads each dependency's `module` or `main` field. The package is then checked with the launcher's own loader, the same check as `dupin ext check`: the manifest is validated in full, and the bundle is compiled and linked and its top-level code run.

</details>

<details>
<summary>What fails a build</summary>

A dependency that imports a Node built-in (`fs`, `net`, `child_process` and the rest, with or without `node:`), by `import`, `import()` or `require`, fails the build, naming the file that imports it. The configuration's resolver hook stops it before Rolldown would keep it as an external import, since the guest has no built-ins. Any other import Rolldown can't resolve, such as a dependency that isn't installed, fails the build too. A failed build or check puts back the `dupin.main` that was there before.

</details>

## Installing from a source

`dupin ext install` takes a `github:` repository, any Git URL or a folder; [Extensions](./extensions.md#installing-and-updating) has the commands.

| Source                                                     | Ref                                                             | Needs   |
| ---------------------------------------------------------- | --------------------------------------------------------------- | ------- |
| `github:owner/repo`                                        | `@tag`, `@branch` or `@<full SHA>`; else the newest version tag | Nothing |
| `https://`, `ssh://` or `file://` URL, or `user@host:path` | The same                                                        | Git     |
| A folder, such as `./word-count`                           | None: it is copied as it is now                                 | Nothing |

> [!NOTE]
> A URL can't carry a password or token, which would end up in the installed extensions list; let Git supply it from a credential helper or an SSH key.

<details>
<summary>How refs resolve</summary>

Without a ref, Dupin installs the highest version tag (`v1.2.3` or `1.2.3`; a pre-release ranks below its release, and `rc.10` above `rc.9`), or the default branch when the repository has no version tag. A commit is named by its full SHA, and must be the repository's own: on a branch or tag of a Git source, or on the default branch or a tag of a `github:` source, since GitHub serves a commit pushed to any fork through the repository it was forked from. `dupin ext sync` also takes the locked commit from the branch or tag it was installed from, so a package installed from another branch restores. A ref such as `refs/pull/1/head`, someone else's proposed commit, is refused.

</details>

<details>
<summary>How each source is fetched</summary>

Installing never runs anything from the package, and there is no package manager, install script or build. `dupin ext update` and `dupin ext sync` fetch the same way.

- **`github:`** needs no developer tools. Dupin asks GitHub's REST API which commit the ref names, downloads GitHub's tarball of exactly that commit with the macOS system curl (`/usr/bin/curl`, ignoring `~/.curlrc`), and unpacks it itself. Every file must match the commit's tree, read from the API first: the same path, mode, size and Git blob id, so a package locks the same as it would from Git. A file that `.gitattributes` drops from archives (`export-ignore`) or rewrites in them (`export-subst`, line endings) fails the install, as does a Git LFS file when the repository's "Include Git LFS objects in archives" setting is on, because the archive then holds the object where the commit holds its pointer. If `GITHUB_TOKEN` (or else `GH_TOKEN`) is set, Dupin sends it to the API, for a private repository or a higher rate limit than GitHub's 60 requests an hour; it is never required, and never sent to the host the archive comes from.
- **A Git URL** runs `git` to fetch the one commit into a scratch repository, and Dupin reads that commit's files itself: nothing is checked out, so hooks, filters, Git LFS and submodules never run. Without Git, the install fails with a message instead of running `/usr/bin/git`, which on a Mac without the Command Line Tools only opens their installer.
- **A folder** needs neither. It skips `.git` and its top-level `node_modules`.

</details>

### What an install checks

Before placing anything, Dupin checks that:

- `package.json` has a valid `dupin` key and `name`;
- `dupin.main` (default `main.js`) is a `.js`, `.mjs`, `.ts`, `.mts` or `.tsx` file in the package, not a declaration file. A JSX entry, or a build output that isn't committed, fails with a hint to build and commit it;
- there are no symbolic links, submodules or `.git` folders, and no two names that differ only in case;
- there are at most 1,024 files, 4 MiB per file and 16 MiB in all, with paths up to 512 bytes and 16 folders deep. A fetch or download of more than 64 MiB is stopped before it finishes, and a tarball whose entries aren't plain files and folders inside its one top-level folder is refused;
- the keyword isn't already used by a built-in command, a web search keyword or another folder in the extensions folder;
- a folder with the package's name, if there is one, was installed by `dupin ext install` from the same source. Grants belong to that source, so installing another package under an installed name fails until you remove the old one.

<details>
<summary>How it is placed</summary>

The package is written into a hidden folder beside the others and then renamed into place, swapping with the previous copy on an update, so a running Dupin sees the old package or the new one and never half of either. It runs once the [extensions folder](./extensions.md#turning-them-on) is on, like any other folder there. Two commands that change extensions at the same time run one after the other.

</details>

## Finding packages

`dupin ext search [QUERY]` lists the public GitHub repositories with the `dupin-extension` topic, most stars first, through GitHub's search API. Words after `search` narrow it, as GitHub's repository search reads them:

```sh
dupin ext search            # every tagged repository, the 20 with the most stars
dupin ext search clipboard  # those that also match "clipboard"
```

Each listing shows the repository's owner and name, description, stars, the day of its last push, what its `package.json` on the default branch declares (title, version, name, keyword and the capability scopes it requests) and the exact `dupin ext install github:owner/repo` line to install it. `--json` returns the same listing with `"reviewed": false`.

> [!NOTE]
> Listings are not reviewed: anyone can add the topic. The [capability broker](./extension-permissions.md), not a review, decides what a package can reach, and installing asks nothing on your behalf: you choose what to grant when you turn it on. The listing reads the default branch, and an install without a ref takes the newest version tag, so check what `dupin ext install` reports it requests.

<details>
<summary>Security details</summary>

Searching never installs anything. Dupin treats everything GitHub and the repositories send as untrusted text: control characters, terminal escape sequences and bidirectional or invisible formatting characters are dropped, each field is one line of bounded length, and a `package.json` larger than 64 KiB isn't read. A repository GitHub lists without the topic, or under a name `dupin ext install` wouldn't take, is left out.

The search goes through the macOS system curl, as a `github:` install does. GitHub allows 10 searches a minute without a token; if `GITHUB_TOKEN` (or else `GH_TOKEN`) is set, Dupin sends it to the API host for a higher limit, and never prints it. Each `package.json` is read from `raw.githubusercontent.com` without the token.

</details>

## The installed extensions list

Dupin pins what `dupin ext install` placed in its [encrypted storage](./storage.md#built-in-namespaces), one record per package. `dupin ext list` shows it. At most 256 packages can be installed.

| Namespace          | Holds                                                                    |
| ------------------ | ------------------------------------------------------------------------ |
| `extensions`       | A package from GitHub or Git.                                            |
| `extensions-local` | A package copied from a folder, whose path means nothing on another Mac. |

```json
{
	"name": "word-count",
	"source": "github:someone/dupin-word-count",
	"ref": "v1.2.0",
	"commit": "0123456789abcdef0123456789abcdef01234567",
	"content_hash": "blake3:…",
	"installed_at": "2026-11-23T09:00:00Z",
	"version": "1.2.0",
	"keyword": "wc",
	"requested": ["http:https://api.example.com"]
}
```

`content_hash` covers every file's path, executable bit and bytes. `requested` lists the scopes `dupin.capabilities` declares, named as the [capability broker](../capability-broker.md) names grants: an `http` entry for `https://api.example.com` requests `http:https://api.example.com`, a `files` entry named `notes` requests `files:notes`, and `"clipboard": ["write"]` requests `clipboard:write`. A declaration the broker rejects stops the install.

> [!NOTE]
> Reproducing a set of extensions on another Mac waits for [sync](../design/sync.md#extensions), which will carry the `extensions` records and reinstall each package from its pinned commit.

<details>
<summary>Why it is encrypted, and who writes it</summary>

A source names a repository, maybe a private one, or a folder of yours, so the list is never written in the clear. While Dupin runs it holds the list, and `dupin ext` asks it over the control socket to read or change it; otherwise the command opens the list itself.

</details>

<details>
<summary>Grants and updates</summary>

The list holds no grants: what you allow is in the `extension-grants` namespace, which the broker owns. When Dupin loads an updated package, its grants of scopes the new revision declares no wider carry over, scopes it widens or adds wait for you to allow them, and scopes it drops are forgotten; `dupin ext update` lists what it adds and drops. `dupin ext remove` forgets the package's grants too, and so does installing a name that wasn't installed, so a package never inherits what you allowed another of that name. While Dupin runs, the command asks it to forget them, so it neither keeps nor writes back what the command forgot.

</details>

<details>
<summary>Integrity checks</summary>

When Dupin loads the extensions folder, it checks each listed package against its content hash, rehashing only after the package's files change. A package whose files differ, or that gained a symbolic link, is skipped and Settings → Extensions shows it as **disabled**; a list that can't be read, because the Keychain is locked or it was sealed with another key, keeps every extension off, and Settings says why on every page. `dupin ext sync` puts the installed copy back. Folders the list doesn't name, such as ones you made yourself, aren't checked.

</details>
