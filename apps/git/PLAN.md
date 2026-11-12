# Git implementation plan

Native Linux Git client · Rust · GTK4 · Omarchy / Arch Linux

## Scope

A free, offline-capable Git client in the Nyx suite, with a GitKraken-class commit
graph, safe Git workflows, conflict resolution, worktrees, multiple repositories and
optional GitHub, GitLab, Forgejo and Gitea integration. No IDE, code editor, AI
features, embedded terminal, custom server or proprietary cloud dependency.

The original project specification supplied for this implementation governs the
later phases; the checklist below tracks delivery, rather than claiming roadmap
features are implemented.

## Phase 0: Discovery — complete

- [x] Read repository `STYLE.md` and parent style guidance.
- [x] Inspect Music's window, resize handling, theme and settings implementations.
- [x] Inspect Contacts' application identity, persistence and corrected navigation.
- [x] Inspect shared `nyx-core`, `nyx-theme`, `nyx-ui` and `nyx-icons`.
- [x] Identify suite installer, launcher and Hyprland conventions.
- [x] Validate GTK4, GtkSourceView 5, system Git and Rust dependencies locally.

### Architecture and style decisions

The app lives in `apps/git`, auto-enrolled by the workspace's `apps/*` membership.
`src/core` is a reusable library independent of GTK: gitoxide discovers repositories
and reads HEAD; `operations` centralizes system Git execution with typed operations,
argument arrays and actionable errors; `status` parses NUL-delimited porcelain v2.
CLI status is used for native Git compatibility. Future object/ref/history readers
belong in this core; CLI mutations must extend the centralized runner.

`src/app.rs` runs serialized inspection jobs through the existing
`nyx_core::cmd::background` worker facility. A later repository choice supersedes
queued refreshes. No Git subprocess or discovery scan runs on GTK's main thread.
GTK ListView recycles visible file rows. Provider and graph abstractions are deferred
until needed; no speculative empty modules or nonfunctional action buttons.

Reuse shared theme installation, suite theme polling, XDG preferences, widgets,
settings card, sidebar head, common keys and icon template. Flat semantic surfaces,
JetBrains Mono, hairlines, shared hover and selection states, no title bar or top
bar. Navigation/list/scroller explicitly disable horizontal expansion. CSS owns
sidebar widths; compact mode follows Music's allocation probe plus slow fallback.
File details use a resizable split, vertical in narrow windows. Navigation itself
is fixed-width per STYLE.md, taking precedence over the generic resizable-panel goal.
Hyprland uses the existing suite's tiled behavior; no custom window rule is needed.
Ctrl+Shift+P stays the suite palette; the suggested Push binding must change later.

## Phase 1: Application foundation — implemented

- [x] Rust GTK4 application, suite identity and single-instance command handling.
- [x] Shared Nyx styling, live themes, manual choice and settings card.
- [x] Main window, fixed-width responsive sidebar and file details.
- [x] Bounded folder discovery and opening repositories/subfolders/worktrees.
- [x] Accurate NUL-delimited status, staged/unstaged/untracked/conflict classification.
- [x] Empty/unborn, detached HEAD and bare repository support.
- [x] Settings persistence, recent repositories, optional last-repository restoration.
- [x] Background loading and structured logging.
- [x] Launcher metadata, shared app icon and installer catalog registration.
- [x] Targeted parser/argument and temporary-repository integration tests.

Validation: `cargo test -p nyx-git` passed (2 unit tests, 3 temporary-repository
integration tests); the GTK binary compiled, installer scripts passed `bash -n`,
and the installer catalog lists Git. Interactive visual QA is still pending.

Exit: opens real repositories and displays system Git's status without modifying
repository content. This is a foundation release, not the complete Git client.

## Phase 2: Core Git workflow — implemented

- [x] Whole-file and all-file staging/unstaging; staged/unstaged/untracked filters.
- [x] GtkSourceView 5 diff preview with live semantic theme colors.
- [x] Hunk and selected-line staging of regular tracked UTF-8 text modifications.
- [x] Commit composer, amend confirmation and last-message loading.
- [x] Fetch, fast-forward pull, push and branch push/upstream selection.
- [x] Create/switch/rename/delete merged local branches; Git refuses unsafe changes.
- [x] Stash save/list/inspect/apply/pop/drop with stale-list and drop safeguards.
- [x] Native hooks, signing, helpers and SSH agents; actionable remote error messages.
- [x] Serialize mutations with status loading; refresh after success and failure.
- [x] Keep commit text after failure and retain per-repository drafts while switching.
- [x] Reject stale selective diffs and refuse to silently stage conflicted files.

Validation: 2 unit and 6 temporary-repository integration tests pass, covering
literal filenames, unborn unstaging, commit/amend, failed hooks, partial staging,
stale patches, unmerged branch protection, stash round trips and local-only remotes.

Limits: partial staging deliberately excludes binary/non-UTF-8/new/deleted/renamed/
mode-changing/unterminated files; whole-file staging handles these. Conflict
resolution/staging is provided by the Phase 4 editor. Remote configuration, cloning,
transfer cancellation/progress and file monitoring are now provided in Phase 7. GUI
interaction and accessibility review remain part of release validation.

Exit: daily Git work without a terminal.

## Phase 3: Interactive commit graph — implemented

- [x] gitoxide topological traversal, including timestamp skew and shallow boundaries.
- [x] Stable lane allocation and disk-cached graph rows; merge/octopus edges.
- [x] GTK DrawingArea/Cairo rendering with Pango text, theme-derived lane colors,
      local/remote/tag/HEAD labels, current-branch ancestry accent and hover details.
- [x] Commit selection, metadata, changed-file selection and first-parent diffs.
- [x] Keyboard/page navigation, oldest/newest jumps, persistent zoom and copy-hash.
- [x] Context actions and comparisons between two selected commits.
- [x] Branch/tag ancestry scopes; message/hash/ref search plus author/date tokens.
      Search dims nonmatches and navigates between matches, preserving the DAG.
- [x] Incremental background paging, cancellable traversal/search, bounded viewport
      row cache, viewport-only drawing and fractional trackpad scrolling.
- [x] Reproducible 100,000-commit traversal/layout/cache benchmark.
- [x] GTK startup and visual checks at 1120×800 and 800×800; compact navigation,
      vertical details in narrow windows, themed GtkSourceView gutter.

Validation: 11 targeted tests pass (2 unit, 9 integration). Graph cases include
merge/octopus lane continuity, page boundaries, refs/annotated tags, timestamp skew,
shallow roots, search parsing and cancellation. Normal/narrow GTK snapshots checked.

Measured on this machine, 2026-10-17, a synthetic linear 100,000-commit repository:
first 128 rows including topological initialization 395.8 ms; all traversal/layout
791.9 ms; cached viewport read average 0.067 ms; 72 rows per sampled 800 px viewport;
UI cache cap 256 rows; benchmark process RSS about 47.5 MiB. Run:
`cargo run --release -p nyx-git --example graph_benchmark -- 100000`.
These measure data/layout/cache work, not compositor frame rate. Broader scrolling,
GPU/high-DPI and accessibility acceptance remains in Phase 7.

History opens beside a separate working-changes mode. Temporary graph files are
anonymous and removed automatically; compact offsets and traversal state live in
the worker, while only viewport rows enter UI memory. Long search/oldest jumps can
scan the whole selected ancestry in the worker; Stop cancels without changing Git.

Exit: smooth, accurate graph across complex merge histories.

## Phase 4: Advanced Git — implemented, with scoped limits

- [x] Merge and topology-preserving rebase through native Git.
- [x] Interactive linear rebase (up to 500 commits): reorder/pick/squash/fixup/drop.
- [x] Cherry-pick/revert, merge mainline selection, squash with failed-commit rollback.
- [x] Annotated tag creation/deletion and explicit unmerged local branch deletion.
- [x] Soft/mixed/hard reset, per-file discard, fresh-state guards and confirmations.
- [x] Recovery refs and HEAD reflog listing; create recovery branches without checkout.
- [x] Force push with an exact observed remote lease; stale remote updates are rejected.
- [x] GtkSourceView source/result comparison, selectable base/ours/theirs, whole-side
      and selected-text copying, manual editing, highlighted marker navigation.
- [x] Explicit save/stage, marker/index/working-file validation, binary side/deletion
      choices, merge/rebase/cherry-pick/revert continue/abort controls.
- [x] Failed resolution keeps the editor draft; live semantic theme updates.

Validation: 16 package tests pass (2 unit, 14 integration), including real merge and
rebase conflicts, binary resolution, stale file and reset guards, cherry-pick,
revert, abort/continue, recovery/tag/branch operations, literal discard paths,
interactive reorder/fixup/drop, squash hook failure rollback, and exact force-lease
rejection against a local bare remote. GTK conflict-editor preview checked.

Limits: interactive plans and squash require a linear range; no arbitrary exec
commands or merge-todo editing. Conflict editor supports regular text/binary files
up to 16 MiB; symlinks, submodules and larger files use external tools. Continue
uses Git's prepared message; edit it externally if needed. Recovery references
retain committed objects, never discarded uncommitted contents. Normal Git hooks,
signing and credential helpers remain in use. Phase 7 adds progress, transfer
cancellation and common-directory locks; exhaustive interruption coverage remains
release work. No user repositories were mutated in validation.

Never silently discard work or automatically resolve conflicts.

## Phase 5: Worktrees and multiple repositories — implemented

- [x] Native worktree listing with branch/path, clean/dirty/ignored, locked and
      missing/prunable states; create new/existing branch checkouts and open tabs.
- [x] Lock/unlock, guarded clean removal and stale metadata pruning. No force removal;
      main/active/locked/dirty/ignored/unavailable worktrees cannot be removed.
- [x] Revalidate worktree identity, index/files and branch tips before mutations;
      retain removal/pruning commit tips in Nyx recovery refs, including detached HEAD.
- [x] Scrollable repository tabs, close/middle-close, Ctrl+W and Ctrl+(Shift+)Tab.
      Opening multiple repository arguments queues each inspection without dropping tabs.
- [x] Searchable repository workspace (Ctrl+K), favorites and named sidebar groups.
- [x] Restore open tabs and active repository when suite Open on is Last; inactive
      repositories load on demand. Commit drafts/amend state survive closing/reopening.
- [x] Per-repository History/Changes view, zoom, normal/narrow panel positions and drafts.
- [x] Preferred file manager, Omarchy editor and default terminal integration; optional
      VISUAL/EDITOR/TERMINAL command arguments are parsed and executed without a shell.

Validation: four focused temporary-repository tests cover new/existing branch creation,
branch reuse/movement refusal, dirty/ignored/stale removal guards, locking, native prune,
locked stale retention, detached commit recovery, raw NUL-delimited paths and serialized
workspace/draft restoration. GTK checks cover two queued tabs, active-tab/draft persistence,
restart restoration and the worktree manager at narrow width.

Limits: destructive worktree removal is intentionally limited to clean directories,
including no ignored files. Preserve local data externally before removal. Worktrees
must be created outside existing checkout directories; existing directories are refused.
Prune removes Git metadata only, after confirmation; disconnected drives should be locked.
Tabs load inactive repositories on demand. Phase 7 adds independent repository jobs,
shared-worktree serialization and watching; full interruption coverage remains release work. External
application launch uses system preferences; install/configure an editor or terminal if
none is available. No user repositories were mutated in validation.

## Phase 6: Hosting providers — initial workflows implemented

- [x] Optional GitHub, GitLab, Forgejo and Gitea adapters, including custom HTTPS
      instances and instance path prefixes; no background account/network requests.
- [x] Verify accounts against the provider; read existing Git helper credentials or
      save dedicated API tokens through libsecret's secret-tool / Secret Service.
      Settings retain account metadata only. Disconnect clears dedicated tokens.
- [x] Paged repository browsing/filtering, repository links and SSH/HTTPS cloning.
      Cloning arbitrary URLs also works without a hosting account.
- [x] Open/all-state pull/merge request lists, request details and refreshed server
      mergeability/state, source/target/hash/draft metadata; paged repository issues.
- [x] Explicit request publication after a populated form and final confirmation.
      Drafts use provider support or WIP/Draft title conventions as appropriate.
- [x] Request checkout fetches a provider request ref into a private Nyx ref,
      verifies the observed commit, requires a matching local remote and clean state,
      and creates a new local branch without overwriting existing branches.
- [x] HTTPS-only API credentials, sensitive authorization headers, refused redirects,
      bounded responses/timeouts and redacted errors; browser links stay on the account
      origin. Git clone/fetch uses native credentials, not API token injection.

Validation: four focused tests pass. Mock HTTP fixtures verify all four adapters'
endpoints/auth headers/pagination and publication payloads, host/secret validation,
redaction and redirect refusal. Temporary Git repositories verify native request
fetch/checkout, exact-head/matching-remote/dirty-state rejection and clone URL/path
safeguards. GTK connection-dialog snapshot checked with isolated settings; no live
accounts were accessed, no keyring entries were written, and no requests were published.

Limits: live-provider permissions, organization authorization and keyring unlock flows
need manual validation with user-connected accounts. Account connection uses existing
helper tokens or personal API tokens; OAuth/device registration remains future work.
Private clone/fetch requires native Git/SSH credentials even when an API token is saved.
Lists reflect token access (GitLab lists member projects) and load pages on demand.
Status shows server request/mergeability state, not a full approvals/CI review console.
GitLab request creation currently targets branches within the selected project; fork
source-project selection is not implemented. Forgejo/Gitea drafts use a WIP title;
server-specific conventions may vary. HTTPS must have a trusted certificate; arbitrary
HTTP instances and TLS verification bypasses are not supported. Phase 7 adds clone/fetch
transfer controls; provider API cancellation and broader provider/version acceptance
remain release work.

Local Git remains usable without an online account.

## Phase 7: Polish and release — in progress

- [x] Native direct-submodule manager: indexed/current commits, branch and local
      state, declared URLs with credential-bearing URLs hidden, open as repository,
      explicit initialize/update to the indexed commit and native URL synchronization.
- [x] Parent/child guards and common-directory locks; reject stale declarations,
      configuration, index/HEAD/working-state changes, conflicts and symlink paths.
      Updates reject child local/ignored changes and nonempty uninitialized paths;
      retain an initialized prior HEAD in the child’s Recovery before moving it.
      Native checkout uses literal paths and never force, recursive or remote-tip mode.
- [x] Two focused integration tests cover wildcard/Unicode module paths, isolation
      from a similarly named dirty module, indexed checkout/recovery, cached init,
      native URL sync and dirty/ignored/stale/nonempty safeguards. Parent staging
      and HEAD remain intact during updates. GTK manager preview checked.

Submodule limits: working repositories only, at most 256 direct indexed modules.
Add/remove/deinit, recursive updates and remote-tip tracking remain external Git
workflows; nested modules can be managed by opening their child repository. Ignored
files conservatively block checkout updates. Native fetches use existing remotes,
helpers and Git transport policy. No forced cancellation or partial-directory cleanup.

- [x] Collapsible sidebar groups for local/remote branches, tags, stashes, worktrees
      and remotes; current branch/upstream hints, loaded-item filtering and section
      persistence per repository. Reuse Nyx navigation styling and compact behavior.
- [x] Branch/tag selection scopes history without checkout; explicit keyboard/context
      actions reuse Git safeguards, stash inspection, worktree opening and selected
      remote configuration. F6 targets the expanded sidebar filter.
- [x] Background inventories reload on root/HEAD/ref/config/worktree-metadata changes;
      ordinary file refreshes reuse them and preserve navigation focus. Generation/root
      checks reject stale worker results. Bounds: 2,000 loaded items per group and
      100 rendered matches per group, with omission notices. No network requests.
- [x] Focused inventory test covers Unicode branches, upstream/current markers,
      annotated/non-commit tags, symbolic remote HEAD omission, stashes/worktrees/
      remotes, caps, persistence and unchanged HEAD/index/files. Watch plans also
      include worktree metadata under the existing 128-directory metadata budget.
- [x] Hide visible scrollbars across Git views/dialogs while retaining native
      scrolling and keyboard navigation; graph scrolling keeps its adjustment.
- [x] Persist Show scrollbars in Git window settings, hidden by default. Toggle
      existing views/dialogs immediately; new views use the saved preference.
- [x] File history from Changes and commit details, pinned to HEAD/selected commit,
      with native rename following, 50-row pages, first-parent/root/rename diffs,
      commit-hash copying, retry and cancellation on Stop/close. Reuse GtkSourceView
      and suite file-list styling; preserve raw Linux path bytes in the core.
- [x] Targeted tests cover rename pagination with raw-byte/wildcard/newline paths,
      pinning despite HEAD changes, root/deletion diffs, empty/unborn/shallow states,
      invalid paths/page limits, cancellation, and unchanged HEAD/index/working files.
      GTK preview confirms populated history and initial diff rendering.

File-history limits: native Git rename heuristics/history simplification apply;
diffs compare the first parent and retain the existing 2 MiB preview limit. Git's
`--skip` bypasses rename processing, so older pages replay and discard prefix records;
only the requested page's metadata is retained. Command output uses the shared
128 MiB bound; deeper file histories may be slower and remain cancellable.

- [x] Initialize new/existing directories with a chosen initial branch, without
      staging or replacing files. Refuse reinitialization, nesting, final symlinks
      and existing/broken Git metadata; exclusive metadata-directory reservation
      rejects concurrent initialization. Failures retain partial metadata.
- [x] Server-branch listing/deletion from Remotes explicitly uses the single push
      destination. Confirmation names the branch/repository/tip; fetch that tip to
      Recovery, verify it and push deletion with an exact-tip lease. Refuse stale
      local configuration/tracking refs and default/unknown-default deletion.
- [x] Two focused local-only tests verify initialization preservation/refusals,
      distinct fetch/push destinations, server-only recovery, default protection,
      a real pre-push race rejecting deletion, and unchanged local HEAD/files.
- [x] Native Remotes dialog: add, rename, fetch/push URL changes, restore inherited
      push destination, and confirmed removal with recovery refs for tracking tips.
      Native commands run through shared common-directory locks and operation history;
      configuration/tracking-ref snapshots reject stale forms. Bare repositories work.
- [x] Remote URLs use HTTPS/SSH/absolute local paths with credential-bearing URLs
      hidden. Inherited/included definitions are read-only; multivalue URL edits and
      mirror/custom-fetch removal defer to Git. No remote-settings network requests.
- [x] Two targeted temporary-repository tests cover native edits/upstream rename,
      retaining a remote-only commit on removal, unchanged HEAD/working contents,
      literal local paths, stale/unsafe/multivalue/custom-fetch/included safeguards.
- [x] Central Git job controls: bounded transfer progress, session operation history,
      explicit failed-fetch retry and cancellable queued/running fetches and clones.
- [x] Common Git directory locks serialize Nyx mutations across linked worktrees and
      processes; independent repositories can run operations concurrently. Fetches wait
      for the shared lock and can be stopped while queued; other conflicting changes
      are refused. Native Git's own locks continue to protect against external tools.
- [x] Process groups stop Git and transport children together, with graceful interrupt
      followed by termination if needed; mutating checkout/commit/rebase operations are
      not offered unsafe forced cancellation. Partial transfer results remain inspectable.
- [x] Switching/closing tabs preserves job context, results and per-repository drafts;
      completing a background job never replaces the active unrelated repository.
- [x] Block ordinary quit while Git jobs remain active; background completion notifications
      identify the operation/repository without exposing full error or credential details.
- [x] Bounded GIO directory monitoring, 650 ms debounce and refresh deferred until a
      shared-repository mutation finishes. Watch tracked and nonignored path parents,
      per-worktree metadata and shared refs, without walking ignored cache trees.
- [x] Reuse unchanged watch plans; watch at most 640 directories (up to 128 metadata/ref
      directories) with 30-second fallback when limited/unavailable or using submodules.
      Reinstall directory monitors after replacement, and recheck once after installation.
- [x] Working-file/index changes refresh status/diffs without resetting history; ref,
      HEAD, configuration and shallow-boundary changes reload history. F5 remains manual refresh.
- [x] 27 package tests pass, including process-tree cancellation preserving local files/
      index/HEAD, shared worktree and cross-process locks, cancellable lock waiting,
      independent repository access and watch plans excluding ignored caches. The
      ignored helper test is invoked explicitly by the cross-process test.
- [x] GTK checks with temporary repositories: stop a transfer while another tab remains
      active; retain context in operation history; refresh an external nested-file edit.
- [x] Keyboard commit actions with Shift+F10/Menu, Enter to focus details, and
      F6/Shift+F6 panel navigation. Focused popovers handle Escape before page search.
- [x] Accessible graph selection summaries (position/hash/message/author/date/refs/
      parent count/shallow boundary) with low-priority announcements only while the
      graph has focus; named ancestry/search/message/changed-file/diff controls.
- [x] Scrollable details preserve room for history in short tiled windows. Targeted
      GTK checks at 1120×800 and 800×500 navigate panels with F6, select the next
      commit and open Shift+F10 actions with focus on Copy commit hash. Build passes;
      live screen-reader announcements remain unverified (desktop AT-SPI bus unavailable).
- [ ] Full screen-reader keyboard/accessibility, high-DPI and live-theme acceptance;
      the Cairo graph exposes its current selection, not a semantic tree of all rows.
- [ ] Updated startup/scrolling/memory measurements and wider hardware testing.
- [ ] Live account/keyring/provider-version acceptance and API cancellation.
- [ ] Arch packaging/release candidate testing and exhaustive interruption/auth/permission cases.

Limits: operation history is session-only (up to 100 records), at most 32 jobs may be
active, and only fetch/clone/LFS fetch expose cancellation. API requests still use their bounded
HTTP timeouts. No automatic mutation retries, lock-file deletion, recovery resets or
partial-clone cleanup occur. Advisory Nyx locks cannot prevent unrelated external Git
processes; native locks and fresh-state safeguards remain necessary. Full arbitrary
mutation queueing and cancellation are deliberately not claimed. Existing graph
benchmarks remain data/layout/cache measurements, not current compositor FPS results.
This is a working development build; Phase 7 release acceptance is not complete.

Definition of done: suite visual compliance, complete safe everyday/advanced Git,
accurate scalable graph, visual conflict resolution, first-class worktrees and
multi-repo workflows, optional four-provider integrations and daily-use stability.


### Git LFS increment

- [x] Native LFS file/pattern inspection; repository-local filter/hook setup without
      force; explicit track/untrack glob controls and current-commit object fetch.
- [x] Central operation locking/progress and fetch cancellation; fresh repository,
      configuration, .gitattributes and .lfsconfig checks; reject symbolic-link
      declarations. No automatic staging, renormalization, migration or pruning.
- [x] Installer includes git-lfs; optional core/provider independence is preserved.
- [x] Focused native integration test: Unicode file tracking, local-remote object
      fetch, index/HEAD/content preservation, stale attributes and symlink refusal.
- [x] Confirmed cached-object checkout through the central operation layer; hydrate
      missing files and matching pointers without downloads or automatic staging.
      Focused coverage checks pointer hydration, modified-file preservation,
      stale working state, deleted-file restoration and unchanged index entries/HEAD.
- [ ] LFS migration, server locking and live hosting acceptance.


### Arch packaging increment

- [x] Local locked-Cargo build and checksummed makepkg binary snapshot; package owns
      /usr/bin binary, launcher, icon, MIT license and app documentation.
- [x] Required native runtime dependencies and optional LFS/keyring integrations.
- [x] Reversible user-local file migration after successful pacman installation;
      checkout installer and suite uninstaller respect package ownership.
- [ ] Published source/release packaging, clean-chroot and full release acceptance.


### UX and code review increment

- [x] Background refreshes no longer disable the window: the file list updates in place,
      the selected file, diff scroll and commit-message focus stay put; an action
      overtakes a status read already under way. Watch plans are rebuilt only when a
      directory appears or goes.
- [x] History reloads keep the selected commit and scroll position (bounded `Locate`);
      the graph polls its worker only while an answer is due; theme changes arrive by
      `nyx_theme::subscribe` instead of timers.
- [x] Commits keep `#` lines (`--cleanup=whitespace`). Transfer failures get a next
      step only when Git's message points at one; Push publishes a branch without an
      upstream. Commit needs staged changes (or Amend) and shows a 50/72 counter.
- [x] One place per action: Fetch/Pull/Push with ahead/behind in the toolbar, a
      repository menu for the managers, a + menu for opening repositories; the
      sidebar holds branches and repositories. Commit actions sit in the graph's
      context menu (check out, branch, tag, merge, rebase, cherry-pick, revert,
      reset); Ctrl+click compares. Commit files are a list; diffs show file line
      numbers and tinted changes. Errors stay until dismissed.
- [x] Control interface for Nyx Assistant: `repo.open`, `repo.status`, `repo.log`,
      `repo.fetch` (not yet listed in the Assistant's nyx-apps ability).


## Nyx Git 2.0 — implemented

The redesign kept the core and the existing workflows and added, in this order:

- [x] Five destinations in the header (Flow, Changes, History, Review, Activity), typed
      command palette and quick switchers, persisted pane sizes.
- [x] Changes: side-by-side and word-level diffs (hunk and line staging stay in the
      unified view), blame, commit templates, rename and binary display.
- [x] History: first-parent collapse and a Timeline over the same rows and edges.
- [x] Operation journal for every core mutation; previews before reset, rebase, merge,
      cherry-pick, revert, deletions, force push, discard, abort and worktree removal;
      Activity merges the journal with HEAD's reflog and recovers by making branches.
- [x] Flow: branches as a deterministic tree by ancestry, worktrees, Nyx Agents sessions
      (from `hub.json`), requests with checks, and derived shared-file warnings, each
      drawn its own way; card positions persist apart from Git.
- [x] Rebase Studio (reorder, reword, edit, split, squash, fixup, drop; proposed history,
      published commits, reorder conflict forecast; Skip) and Conflict Studio (Base, Ours,
      Theirs, editable result, per-conflict choices, modify/delete and binary choices).
- [x] Review: requests, checks, reviews or approvals, commits and files for GitHub,
      GitLab, Forgejo and Gitea, checkout for review and server merges at the shown head.
- [x] Control API 2 for `nyx git …`, Glimpse's Git panel and the Assistant.

Validation: 59 package tests (temporary repositories and mock HTTP servers), including
studios, safety, flow, fixtures (bare, unborn, detached, shallow, submodule, 300
branches, mid-merge). `examples/flow_benchmark.rs` on 20,000 commits, 400 branches and
8 worktrees: 384 ms to read, layout under 0.1 ms (this machine, 2026-11-11).

Limits: previews forecast from the repository as it is; they can't promise an operation
won't conflict. Recovery covers committed objects; discarded working-tree contents and
remote changes aren't recoverable from here. Agent data is limited to live sessions in
`hub.json` (title and status, no task details). Providers differ: GitLab reports no line
counts and merges with the project's method. Review wasn't exercised against live
accounts. Flow's layout is a tree by ancestry; other relationships are drawn over it.
