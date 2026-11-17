# Git

A native Rust + GTK4 Git client in the Nyx Suite. The foundation, core workflow,
visual commit graph, advanced Git actions and conflict resolution are implemented.
Worktree management, repository workspaces and optional hosting workflows are implemented.
Release polish remains on the roadmap in [PLAN.md](PLAN.md).

```sh
# Arch dependencies: gtk4 gtksourceview5 libsecret git git-lfs pkgconf gcc rust
cargo run -p nyx-git -- /path/to/repository [/path/to/another]
cargo test -p nyx-git
./install.sh  # From a checkout of this repository
```

For a pacman-managed Arch installation, run from a checkout of this repository:

```sh
# Requires base-devel, rust, python and desktop-file-utils, plus runtime dependencies.
apps/git/packaging/build.sh            # Build target/packages/nyx-git-*.pkg.tar.zst
apps/git/packaging/build.sh --install  # Build and install through pacman
```

The helper builds the current checkout with Cargo's lockfile, then packages a
checksummed binary snapshot with the launcher, icon, license and documentation.
It is a local package recipe, not an AUR recipe or a published source release.
The generated PKGBUILD and input checksums are installed with the documentation.
Git LFS and a Secret Service provider are optional runtime dependencies. Existing
user-local launcher/icon/binary copies are backed up under
`$XDG_STATE_HOME/nyx-git/package-backups` (default `~/.local/state`) after a
successful package installation; a local binary symlink keeps `nyx list` working.
Rebuild with the same helper for updates, or use `./install.sh` from this
checkout. A downloaded suite installer directs package users to the checkout.
Remove with `./uninstall.sh git` or `sudo pacman -R nyx-git`; preferences and
repository data are retained unless the suite uninstaller's `--purge` is requested.
After direct pacman removal the local symlink can be removed separately.

Open a repository or a subfolder with Ctrl+O, or pass it on the command line.
Linked worktrees and bare repository metadata are supported. Ctrl+Shift+O discovers
repositories in a folder and its immediate subfolders, without following symlinks.
Up to 20 recent repositories appear in the sidebar; ones that no longer exist drop off
at startup, and right-clicking one removes it. F5 refreshes external changes.
The expanded sidebar has collapsible local/remote branches, tags, stashes, worktrees
and remotes, with a filter and per-repository collapsed-section preferences.
Click a branch/tag to view ancestry, a stash to inspect it, or a worktree to open it.
Right-click or Shift+F10 opens actions; switching branches remains explicit.
F6 focuses the sidebar filter when the sidebar is expanded. The tree hides with the
compact sidebar, where repository icons remain and the header's repository menu
reaches every manager.
Inventory reads run in the background after repository/ref/config changes, not each
working-file refresh. Each group loads up to 2,000 items and renders at most 100
filtered matches; notices identify omitted entries. No remote network queries occur.
The tab strip's **+** menu opens, clones, initializes or discovers repositories.
**Initialize repository…** creates a repository in a new or existing
directory with your chosen initial branch. Existing files remain untracked. Existing
repositories, nested repositories, final-path symlinks and pre-existing Git metadata
are refused; failed initialization retains partial metadata for inspection.

**History** shows a virtualized commit DAG with local branches, remote refs, tags and
HEAD. Select a commit for its metadata, message, changed files and first-parent diff.
Use the searchable scope list for a branch/tag's ancestry. Search messages, hashes, refs or
authors, with optional `author:name after:2026-01-01 before:2026-12-31` tokens.
Enter/Shift+Enter find the next/previous match while dimming nonmatches and keeping
topology visible. Up/Down/Page Up/Page Down navigate; Home/End jump to newest/oldest.
Ctrl++/Ctrl+-/Ctrl+0 or Ctrl+scroll zoom, Ctrl+C copies the selected hash. Right-click a
commit to check it out, create a branch or tag there, merge, rebase onto it, cherry-pick,
revert or reset to it, or compare. Ctrl+click compares a commit with the selected one.
Shift+F10 or the Menu key opens the selected commit's actions; Enter focuses its
details. F6 / Shift+F6 cycles between sidebar, history/changes and details panels.
The graph exposes the selected commit's position, hash, subject, author, relative
date, refs and parent count to accessibility tools, announcing changes while focused.
Stop cancels a long search or jump to the oldest commit; F5 reloads. A reload after a
ref change keeps the selected commit and scroll position. Older rows load while
scrolling, with metadata and layout cached in an anonymous temporary file and no
more than 256 graph rows retained by the UI. Alt+1 opens History; Alt+2 opens Changes.

**File history…** in a changed file's ⋯ menu or above a commit’s changed-file list
opens that path’s commits and historical diffs. History stays pinned to its opening
revision (the selected commit, or HEAD from Changes), follows Git-detected renames,
and shows 50 commits per page with Newer/Older navigation. Select a row to view its
first-parent diff, including both names for a rename. Copy the commit hash, stop
loading, or reload a failed/stopped page. Uncommitted renames use the original path.
Shallow boundaries and oversized/non-UTF-8 diffs show explanatory messages.
Rename following uses Git’s heuristics and history simplification; deeper pages
replay the traversal from the pinned revision, so they can take longer. Queries
are cancellable and retain bounded UI pages rather than all file-history metadata.

In **Changes**, select a changed file to preview its working diff; **Staged diff** shows the index
against HEAD. Filter the file list by staging state or search paths with Ctrl+F.
The diff gutter shows the file's old and new line numbers. Background refreshes keep
the selected file, the diff's scroll position and the commit message's focus.
Use **Stage file**, **Unstage file**, or their all-file actions. Place the cursor in
a text hunk for **Stage hunk**, or select added/removed lines for **Stage lines**.
Partial staging applies only to regular tracked UTF-8 text modifications with final
newlines. Binary/new/deleted/renamed files use whole-file staging. Diffs exceeding
2 MiB need external inspection. File operations preserve raw Linux filename bytes.

Write a commit message and press Ctrl+Enter. **Commit** is enabled once something is
staged (or **Amend HEAD** is on); the counter shows the summary line's length against
50, red past 72. Lines starting with `#` are kept. **Amend HEAD** asks before replacing
the latest commit; **Last message** copies its message into the editor. Failed
hooks/signing leave the message and changes intact. Drafts survive repository
switches, tab closure and restarts. Git's configuration, hooks, signing, SSH
agents, askpass and credential helpers remain in use; tokens are not stored here.

**Fetch**, **Pull** and **Push** sit in the header in both views and run in background
workers; Pull and Push show how far the branch is behind and ahead, and Push becomes
**Publish** for a branch without an upstream. Pull only fast-forwards; diverged
histories use an explicit merge/rebase from a commit's context menu. Failures say what
to do next only when Git's message points at it. The header's repository menu (☰) opens
**Branches**, **Stashes**, **Remotes**, **Worktrees**, **Submodules**, **Git LFS**, **Git
actions…**, **Recovery points…** and **Operation history…**, and opens the folder in a
file manager, editor or terminal. **Branches**
supports create/switch/rename, deleting merged branches, and pushing a selected
branch to a remote with upstream setup. **Remotes**
adds/renames remotes, changes fetch/push URLs and removes a remote after confirmation.
An empty push URL uses the fetch URL. HTTPS, SSH and absolute local paths are supported;
credential-bearing URLs are hidden and new URLs use native credential helpers/SSH.
Changes reject stale configuration/tracking refs; removal retains tracking tips in
Recovery. Inherited/included configuration, multivalue URL edits and removal of
mirror/custom fetch mappings require Git. These settings do not contact the server.
**Server branches…** explicitly contacts the selected remote’s push destination.
Choose a branch and confirm deletion: Nyx fetches its tip into local Recovery and
deletes only with an exact-tip force-with-lease. Concurrent changes are refused.
The advertised default branch is protected; deletion is unavailable if the server
does not identify it, or if multiple push destinations are configured. Server hooks
and permissions still apply. Local branches and working files remain intact; Fetch
updates stale remote-tracking refs according to your Git prune configuration.
**Stashes** saves tracked/untracked changes, previews saved diffs, applies, pops or
drops stashes. Drop asks for confirmation; stale stash-list entries are rejected.
Failed operations refresh status, including conflicts created by Git.

**Git LFS** lists tracked files and tracking patterns. Enable LFS
explicitly for the current repository (local filters and native pre-push hook),
track/untrack a glob pattern, or fetch the current commit’s objects from a chosen
remote. Review and stage `.gitattributes` yourself; existing files require explicit
restaging to convert, and existing history is never migrated automatically.
Fetch respects native LFS endpoints, credentials and include/exclude settings;
it does not hydrate working files and offers Stop in Git operations.
**Check out cached…** fills missing files and matching pointers from downloaded
objects, preserving modified files through native Git LFS. Deleted LFS files may
reappear; unavailable objects remain missing or as pointers. Checkout downloads
nothing and stages nothing. See [LFS checkout](https://github.com/git-lfs/git-lfs/blob/main/docs/man/git-lfs-checkout.adoc).
Configuration,
HEAD/index/working state and root attribute declarations are checked before actions;
existing hooks are not forcibly replaced. Failed setup can leave partial native
configuration. Install `git-lfs` for these optional controls; local Git works without
an LFS account. See the [native Git LFS documentation](https://github.com/git-lfs/git-lfs/tree/main/docs/man).

**Submodules** lists direct indexed submodules,
their recorded/checked-out commits, branch, local-change state and declared URL.
Open an initialized submodule as a repository tab, initialize/update it to the
parent index’s commit, or explicitly synchronize its declared URL into local Git
configuration. Native Git handles credentials and transport policy. Updates can
detach HEAD; a previous initialized tip is retained in the child’s **Recovery**.
Updates refuse dirty submodules, ignored contents, nonempty uninitialized
directories, conflicts, symlink paths and stale repository/configuration state.
Parent and initialized-child mutation locks coordinate Nyx operations. No forced,
recursive, remote-tip, add/remove or deinit actions are provided here; open a child
to manage its nested modules. More than 256 direct modules require Git. Partial
initializations remain for inspection after failure; these operations do not expose
forced cancellation. No parent commit or automatic staging occurs.

**Git actions…**, or a commit's context menu, offers check out, branch creation, merge, rebase with merge topology
preserved, cherry-pick/revert (including mainline selection for merge commits),
soft/mixed/hard reset, annotated tags and deletion of unmerged local branches.
Interactive rebase supports reordering, pick, squash, fixup and drop for linear
ranges of up to 500 commits. Squash after a selected base creates one commit;
a failed commit restores the original HEAD. History-changing operations retain
`refs/nyx/recovery/…` points. **Recovery points…** lists these and the HEAD reflog, and
creates a branch without touching working files. Recovery references retain
committed history, not discarded uncommitted contents.

Merge/rebase/cherry-pick/revert require a clean working tree, including untracked
files. Confirmations capture repository state and refuse stale actions. **Force
push with lease…** reads the exact remote tip before confirmation and rejects a
push if that tip changed. **Discard…** restores a selected tracked file from the
index, or deletes the selected untracked file, only after explicit confirmation.
Confirmations for actions that can lose work focus **Not now**; others focus the action.

During a stopped operation, **Resolve conflicts…**, **Continue…** and **Abort…**
appear above the main view. The conflict editor compares a selectable base/ours/
theirs pane beside an editable result. Copy a complete side or selected source
text, edit manually, navigate highlighted conflict markers, then save and stage
explicitly. Remaining markers and external file/index changes block resolution;
a failed save keeps the editor draft open. Rebase labels explain Git's swapped
ours/theirs perspective. Binary conflicts offer explicit whole-side or deletion
choices. Regular text/binary files up to 16 MiB are supported; symlink, submodule
and larger conflicts require an external tool. Continue uses Git's prepared
message; abort warns that resolution edits can be lost. Refresh external changes
with F5 before choosing conflicts. Forced cancellation of mutation commands remains
intentionally unavailable. Nyx coordinates its processes through common-directory locks.

The tab strip opens multiple repositories in one window. Ctrl+Tab / Ctrl+Shift+Tab
switch tabs; Ctrl+W or a tab's close button closes a tab without changing its files.
Commit drafts and amend state are retained. **Repositories** (Ctrl+K) searches paths
and groups, manages favorites/groups, and opens folders in the preferred file manager,
editor or terminal. Omarchy editor defaults and `xdg-terminal-exec` are used where
available; VISUAL/EDITOR/TERMINAL provide fallbacks, parsed without invoking a shell.

**Worktrees** lists branch/path and clean/dirty/ignored, locked and missing states.
Create a new branch checkout or use an existing branch, then open it in a tab.
Destinations must be new directories outside existing checkouts; Git prevents branch
reuse. Lock protects disconnected worktrees. Removal asks for confirmation, preserves
the commit tip in Recovery, and refuses main, active, dirty, ignored, locked or
externally changed checkouts. There is no force removal. Prune previews stale entries,
retains their tips, and removes metadata only; it preserves locked registrations.

The suite's **Open on → Last** restores open tabs and the active repository; other
tabs load when selected. Each repository retains its view, graph zoom, normal/narrow
panel position and commit draft. Opening multiple paths on the command line queues
each repository. Independent repositories can now run Git operations concurrently;
linked worktrees share one mutation lock.

**Hosting accounts…** (the **+** menu) connects GitHub, GitLab, Forgejo and Gitea, including custom
HTTPS instances. Choose an existing Git helper token, or paste a personal API token
into the masked field to store it in the desktop Secret Service keyring. Install
`libsecret` and unlock a Secret Service keyring for dedicated tokens. Settings contain
only account metadata. Account verification checks your login before saving it;
disconnect removes its dedicated secret while preserving native Git credentials.

Browse/filter repositories page by page, open them in a browser, clone with SSH or
HTTPS, list open/all-state requests, refresh request state/mergeability, and open
repository issues. **New request** publishes a title/body/source/target after explicit
confirmation; push the source branch first. GitLab currently supports branches in
the selected project; fork project selection and full approvals/CI views are future
work. Forgejo/Gitea draft requests use a WIP title convention.

**Checkout for review** fetches the provider's request ref and verifies the exact
commit shown, requires a matching local remote and a clean working tree, and creates
a fresh review branch. Select SSH or HTTPS according to your native Git setup. API
tokens are never inserted into Git URLs or command arguments. Private cloning/fetching
still uses native Git helper/SSH credentials. **Clone repository…** in the **+** menu
also accepts an arbitrary SSH/HTTPS URL without any hosting account. Clones require
a new destination directory; failed clones may leave a partial directory to inspect.

No provider calls happen until you choose a hosting action. API redirects are refused,
credentials require HTTPS, browser links stay on the account origin, and response
sizes/timeouts are bounded. Live account permissions and keyring unlock flows still
need validation with connected accounts; development checks use local mock APIs and
temporary Git repositories. OAuth/device connection flows remain future work.

Ctrl+B compacts the sidebar; Ctrl+, opens settings; F1 lists shortcuts.
Ctrl+Shift+F fetches, Ctrl+Shift+L pulls, and Ctrl+Alt+P pushes.
Ctrl+Shift+P remains the suite command palette. The sidebar compacts below 980 px,
with file details moving below the list. Panel position and sidebar preferences
persist. The suite's **Open on** setting controls last-repository restoration.

Short notices fade after a few seconds; errors stay until dismissed with Esc or ×.

Nyx Assistant can work Git over the suite's control interface (`repo.open`,
`repo.status`, `repo.log`, `repo.fetch`). Nothing it offers commits, pushes, discards
or rewrites history.

Themes, live Omarchy synchronization, user palettes, settings cards and widgets use
`nyx-core`, `nyx-theme` and `nyx-ui`. Preferences live in
`$XDG_CONFIG_HOME/nyx-git/settings.toml`. Set `RUST_LOG=nyx_git=debug` for operation
timing logs.

**Operation history…** (repository menu) shows live progress and the last
100 job results for this session. Fetch, clone and LFS fetch offer **Stop**; stopping interrupts
Git and its transport children, retains partial results, and refreshes repository
state. Failed fetches can be retried explicitly. Other repository mutations finish
under their locks; they do not offer forced cancellation. Nyx refuses ordinary quit
while jobs remain active. Background completions use desktop notifications.

You can switch tabs or use another repository while a job runs. Related worktrees
share a common-directory lock across Nyx processes. Fetch can wait for that lock;
other conflicting mutations are refused. Completing a job preserves its original
repository context and the active tab's draft. No mutation is retried automatically.

Active-repository file monitoring refreshes external file/index changes after a
650 ms debounce. Ref/HEAD changes also reload history; ordinary working edits leave
the graph in place. Monitoring excludes ignored cache trees and caps directory
watches, with a 30-second fallback for limited/unavailable watches and submodules.
Refresh waits for related mutations to finish, and F5 remains available. API request
cancellation, complete accessibility/high-DPI acceptance, fresh performance numbers
and release packaging remain follow-up work; see PLAN.md for release limits.

The local synthetic benchmark is reproducible without touching user repositories:
`cargo run --release -p nyx-git --example graph_benchmark -- 100000`.
Its first 128 rows took about 396 ms and all 100,000 layouts took 792 ms on this
machine, with cached viewport reads averaging 0.067 ms. This measures the data and
layout pipeline; it does not measure GUI frame rate.

## Install

On NyxOS, choose Git in Welcome when you set up, or install it later from the Store or with `nyx install nyx-git`. On Omarchy:

```sh
curl -fsSL https://raw.githubusercontent.com/design-nexus/nyx-git/main/install.sh | bash
```

