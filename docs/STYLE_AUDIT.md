# Source style audit

Audited all 37 Rust files under `src/` on 2026-08-20.

This audit uses style demonstrated by the decisions in commit `706c3ec` and the discussion that produced it:

- prefer code that is locally understandable over dense iterator or ownership idioms
- use meaningful local variables
- inline trivial single-use wrappers, but retain helpers that name meaningful operations
- trust successful operation contracts instead of adding duplicate checks
- borrow instead of allocating or cloning when that keeps the code simple
- preserve the boundary between apply-time installation and explicit updates
- keep comments sparse and focused on constraints, invariants, workarounds, and reasons
- write comments directly, use lowercase unless starting with a proper name or acronym, avoid ampersands, and omit terminal punctuation

Behavior-changing suggestions are separated from clear violations. Small allocation improvements that require complicated lifetimes or generic abstractions are excluded.

## Coding violations

| Location | Finding | Smallest correction |
|---|---|---|
| `src/operations/packages/snapd.rs:51-60` | Copies parsed snap names into `String`s although the command output remains alive through sorting and removal. | Store `&str` slices in `names` and pass them directly to `host.run`. |

## Behavioral candidates

These resemble the preferred style but change observable behavior and need an explicit decision.

| Location | Tradeoff |
|---|---|
| `src/operations/host/users.rs:11-20` | Removing the `id -nG` membership query simplifies the function because `groupadd -f` and `usermod -aG` are idempotent, but existing members would still incur CLI validation and privileged commands. |
| `src/operations/toolchains/fnm.rs:42-44` | Removing the post-installer executable check trusts a successful official installer, but defers a missing-output failure to later use. |
| `src/operations/toolchains/rustup.rs:36-38` | Removing the post-installer executable check trusts a successful official installer, but defers a missing-output failure to later use. |
| `src/operations/toolchains/uv.rs:26-28` | Removing the post-installer executable check trusts a successful official installer, but defers a missing-output failure to later use. |
| `src/platform.rs:18-24,34-35,209-218` | Replacing `uname -m` with `std::env::consts::ARCH` removes a subprocess and its parser, but changes detection from kernel architecture to executable target architecture. |

## Comment deletions

These comments narrate obvious code, provide untracked TODOs, or add generic module summaries without useful constraints.

| Location | Comment | Reason |
|---|---|---|
| `src/main.rs:1` | `//! Provision Linux & macOS from one config.` | Generic module summary; also uses an ampersand and terminal punctuation. |
| `src/config.rs:1` | `//! Define & validate Cozydot config.` | Generic module summary. |
| `src/workflow.rs:1` | `//! Derive prerequisites & run each platform's operations in dependency order.` | Restates the module implementation. |
| `src/operations/mod.rs:1` | `//! Execute host operations.` | Restates the module name. |
| `src/config.rs:28` | ``/// Load & validate config at `path`.`` | Restates `Config::load` and its argument. |
| `src/config.rs:77` | ``/// Validate config intent that depends on the detected `platform`.`` | Restates `validate_for_platform`. |
| `src/operations/dotfiles.rs:53` | `// require ~/.gnupg to be a non-symlink dir` | The metadata checks and error already state this. |
| `src/operations/packages/npm.rs:11` | `// get package name without the trailing version / tag` | Narrates the adjacent `rsplit_once` expression. |
| `src/init.rs:211` | `// TODO: do we really need this where we're going?` | Unactionable and provides no constraint. |
| `src/operations/packages/apt/repo.rs:117` | `// TODO: review this` | Unactionable and does not identify what needs review. |

## Comment rewrites

These comments contain useful information but do not match the established wording style.

| Location | Current issue | Suggested wording |
|---|---|---|
| `src/init.rs:20` | Useful non-overwrite guarantee, but uppercase, abbreviated, ampersand, and punctuation. | ``/// create `cozydot.yaml` and the `dotfiles` directory without overwriting user-managed changes`` |
| `src/init.rs:148` | Useful symlink invariant, but uppercase, abbreviated, ampersand, and punctuation. | ``/// create missing directories under `root` and fail if `root` or a child directory is a symlink`` |
| `src/operations/dotfiles.rs:122` | Uses an ampersand and omits an article. | `// rename makes each backup atomic, requiring HOME and XDG_STATE_HOME on the same filesystem` |
| `src/operations/toolchains/go.rs:15` | Uses two ampersands. | `// verify that Go is executable and go version output matches the expected version and platform` |
| `src/operations/desktop/gnome.rs:71` | Uses an ampersand. | `// UUIDs enter request URLs and archive names, so accept only GNOME's path-safe form` |
| `src/operations/packages/snapd.rs:56` | Uses an ampersand. | `// remove app snaps before base and runtime snaps` |
| `src/operations/host/privileged_file.rs:41` | Uses an ampersand. | `// stage beside target for atomic rename, then sync file and parent` |
| `src/operations/desktop/mod.rs:44` | Ends with punctuation. | `// Ubuntu provides this media key; upstream GNOME needs a custom binding` |
| `src/operations/desktop/mod.rs:96` | Starts uppercase and ends with punctuation. | `// complete the binding before publishing its path to GNOME` |
| `src/operations/packages/binary/appimaged.rs:10` | Bare URL does not state why the block exists. | `// appimaged requires removing conflicting tools and stale cache before first launch` |

## CLI help wording

These Rust doc comments become Clap help text. The suggested wording preserves user-facing meaning while matching the direct lowercase style.

| Location | Suggested wording |
|---|---|
| `src/main.rs:13` | `about = "provision Linux and macOS from one active configuration"` |
| `src/main.rs:21` | `/// initialize or synchronize config and bundled dotfiles without overwriting user changes` |
| `src/main.rs:23` | `/// choose config preset` |
| `src/main.rs:27` | `/// check active config` |
| `src/main.rs:29` | `/// apply active config to this host` |
| `src/main.rs:31` | `/// apply configured dotfiles` |
| `src/main.rs:33` | `/// back up conflicts before replacing with Cozydot links` |
| `src/main.rs:37` | `/// run enabled updates` |

Comments not listed above either explain a real constraint or already match the established style.
