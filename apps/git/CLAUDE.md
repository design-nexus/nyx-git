# Git — notes for working on this app

- GTK4 + GtkSourceView 5, no libadwaita, built to `STYLE.md` on Nyx Design 2.0
  (`use_design_2`). App id `io.github.design_nexus.nyx.Git`; settings in
  `~/.config/nyx-git/settings.toml`.
- `src/core` (the `nyx_git` library) has no GTK. System Git is the only writer:
  every mutation is an `operations::Operation` run through `operations::run`/`invoke`
  (structured arguments, literal pathspecs, common-directory locks in `jobs`).
  gitoxide reads HEAD, refs and history. `history::Session` is the graph's worker.
- `src/app.rs`: `Ui::open` (user: disables actions while reading), `Ui::reload` (file
  watcher: quiet, may reuse the watch plan) and `execute_at_then` for operations.
  `loads` drops a status read that an operation overtook.
- `src/ui/window.rs` builds the window and owns `apply` (in-place file list) and notices
  (`nyx_ui::toast`; `error` adds Details with all of Git's text). The header
  (`ui::header`, direction A of `nyx-design/mockups/git`): the repository switcher
  (Ctrl+K) and branch chip (Ctrl+Shift+B), both popovers; the five destinations centred,
  equal segments (`ui::destinations`, Ctrl+1–5, icons only under 1180 px); Sync (one split
  button: Pull, Push, Publish or Fetch by state, ▾ for all), Go to (the palette), ⋯ (F10,
  the repository's tools) and Settings. Neither end may outgrow what the centre leaves it,
  so the destinations never move. Repository tabs (`nyx_ui::tabs`) only with 2+ open.
- Every page starts with `ui::pagebar` (44 px: mode, search with a key hint, actions; its
  start clips rather than widening the window) and inspects with `ui::inspector` (title,
  groups, key/value rows, action rows: one primary, danger in red). Changes is
  `ui::changes` (files grouped by state, a file in both Staged and Changes when it's
  both) beside `ui::workflow` (diff bar, diff, composer card, operation banner).
  The sidebar (`ui::sidebar`): Worktrees, Branches, Remotes, Tags, Stashes, folding,
  `+` to make one. Icons are `git-*-symbolic` (`ui::icons`, from the mockups' set by
  `tools/icons.py`, installed to the cache by `theme::install`).
- Pages (`ui::pages::Pages`, `ui.destinations`): Flow (`ui::flow`, over `core::flow`),
  Review (`ui::review`, over `providers::review`), Activity (`ui::activity`, over
  `core::activity`), and two studios not in the header: Rebase (`ui::rebase_studio`)
  and Conflicts (`ui::conflict_studio`). Only History uses the window's shared details pane.
- Safety: every mutation through `operations::run` is journaled (`core::journal`, under
  `$XDG_STATE_HOME/nyx-git/journal/`, on only when `journal::enable()` ran: the app, not
  tests). History-changing and destructive actions confirm through `ui::preview` over
  `core::preview`. Recovery makes branches; nothing resets automatically.
- Ctrl+Shift+P is the typed palette (`ui::palette`); Ctrl+Alt+W worktrees, Ctrl+P files. Pane sizes persist via `ui::layout`.
  `ui::menu`/`popup_at`/`menu_button` are the app's menus; `ui::dev` holds the
  `NYX_GIT_SNAPSHOT`/`NYX_GIT_PREVIEW_*` developer previews.
- **Control** (`src/control.rs`, API 2, namespace `repo`, alias `git`): status, repos,
  branches, worktrees, changes, history, flow, reviews; open, fetch, switch_branch,
  branch_create, worktree_create. Works headless (`--gapplication-service`, D-Bus
  activation; the installer writes the manifest and service). A `status` topic feeds
  Glimpse's `git` panel (in nyx-glimpse). Nothing commits, pushes, discards or rewrites.
- Agents: `core::agents` reads Nyx Agents' `hub.json` (read-only; `NYX_AGENTS_HUB` points
  a test copy at a scratch file). Sessions match worktrees by working directory.
- Checks: `cargo test -p nyx-git` (temporary repositories only; tests mustn't depend on
  the user's Git config), `cargo clippy -p nyx-git --all-targets` is clean.
- Test window with scratch data: `NYX_GIT_APP_ID=io.github.design_nexus.nyx.GitTest
  GIT_CONFIG_GLOBAL=/dev/null XDG_CONFIG_HOME=… XDG_DATA_HOME=… XDG_CACHE_HOME=…
  XDG_STATE_HOME=… ./target/debug/nyx-git REPO`, on a private headless output (never the
  desktop), captured with `grim -o`. Set `reduce_motion = true` in its settings: a
  headless output barely runs the frame clock, so a view's crossfade never finishes.
  The `NYX_GIT_PREVIEW_*` dialogs still need `NYX_GIT_SNAPSHOT`; never preview dialogs in a
  live test copy (they're separate windows). `NYX_GIT_PREVIEW_VIEW=flow|review|activity|…`,
  `NYX_GIT_PREVIEW_SELECT=0`, `NYX_GIT_PREVIEW_REBASE=<full hash>` and
  `NYX_GIT_PREVIEW_CONFLICTS=1` open main-window pages. This shell may set
  `GIT_CONFIG_COUNT` (core.quotePath); unset it for `cargo test`.
