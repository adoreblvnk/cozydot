# Cozydot capability workflows

This document defines the proposed capability workflow model and order for every Cozydot command.

## Refactor scope

This is a complete refactor of Cozydot's current planning structure, not an additional workflow layer placed on top of
the existing planner. Replace the current structure with the capability workflow model in this document and remove the
obsolete paths after their replacements work end to end.

Do not retain parallel legacy and workflow implementations, compatibility adapters, deprecated planner entry points, or
unused ordering abstractions. Remove superseded planner helpers, old `PlannerPhase` terminology, dead operation-routing
paths, stale tests, and documentation that describes the replaced architecture. Preserve current product behavior unless
this document explicitly changes it, such as the proposed workflow order and the narrower Linux APT update semantics.

Complete each replacement vertically: introduce the workflow, route the command through it, verify its behavior, and then
delete the code it supersedes. The final codebase should contain one obvious implementation path for every command and
capability.

## Implementation intent

Refactor Cozydot around ordered, nested capability workflows while retaining typed operations as the only planned units
that can change configured host state. The goal is to recover the linear readability of the original Bash implementation
without giving up the Rust implementation's validation, safety, idempotence, or cross-platform support.

A contributor should be able to find a capability such as Python and read its complete workflow in one place. Workflows
may compose smaller workflows, and leaf workflows contribute typed operations. For example:

```text
shared
└── tools
    └── python
        ├── bootstrap uv
        └── converge the configured Python toolchain
```

In that example, `shared` and `tools` are composite capability workflows, `python` is a capability workflow, and the two
leaf actions are operations. The implementation does not need a generic runtime workflow tree. Prefer clearly named,
nested planning functions that contribute operations to a complete ordered plan.

### Required architecture

1. Give every CLI command one readable top-level workflow, including `init`, `check`, `apply`, `dotfiles`, and `update`.
2. Deserialize and validate the complete YAML configuration before planning configured host changes.
3. Treat the YAML hierarchy as capability ownership, not execution order.
4. Represent composite capabilities with readable workflow functions that call their child workflows.
5. Represent config-driven, host-changing leaf actions with the closed `Operation` enum.
6. Build the complete operation plan before executing its first operation.
7. Use fixed execution stages to place operations in dependency-safe order regardless of YAML mapping order.
8. Flatten the stages into a sequential `Vec<Operation>` and execute it through the existing typed dispatcher.
9. Keep live host inspection, command execution, filesystem mutation, and postcondition checks in operation executors.

The intended flow is:

```text
YAML
-> validated Config
-> nested capability workflows
-> fixed execution stages
-> ordered Vec<Operation>
-> operation executors
-> host
```

Execution stages are internal ordering buckets. They are not capability workflows and should not be presented as though
they were complete user-facing behaviors. Rename `PlannerPhase` if necessary so this distinction is clear.

Not every command needs an operation plan. `check` is a read-only validation workflow. `init` is a fixed initialization
and publication workflow that runs before an active configuration exists. Keep these commands concrete and linear rather
than forcing them through the config-driven `Operation` planner.

### Ordering intent

The order of mappings in `cozydot.yaml` must not affect execution. In particular, `shared` appearing before `os` must not
cause shared tools to run before platform system setup. System and native package workflows run first, followed by shared
tools and packages, then binaries, fonts, dotfiles, integrations, and desktop behavior as specified below.

Order inside an explicit YAML sequence may be retained where meaningful, such as configured repository definitions or
package names within one operation.

### Command semantics

`cozydot init` owns safe materialization of an embedded preset and bundled dotfiles. It preserves user-edited and
unmanaged files and updates the managed-file manifest only through its fixed initialization workflow.

`cozydot check` owns host-independent parsing and validation of the active configuration. It does not detect the platform,
plan host operations, or mutate the host.

`cozydot apply` owns configuration convergence. It may configure package sources, install missing configured packages,
derive and install prerequisites, bootstrap managers, and apply configured host state.

`cozydot dotfiles` owns standalone convergence of the shared and current-platform dotfile workflows. It validates the
configuration and platform before mutation, preserves conflicts by default, and replaces them only when explicitly
requested.

`cozydot update` owns explicitly requested updates to existing managed state. It must not silently perform the apply
workflow. On Linux, the APT update workflow should refresh metadata and run the configured standard or full upgrade. It
should not republish repositories, install missing configured packages, or process repository conflicts. A user who
changes package or repository configuration runs `cozydot apply` before `cozydot update`.

Keep ensure and update behavior separate. An apply workflow must not update a present manager or tool merely because a
newer release exists. Update behavior runs only when its corresponding update control is enabled.

### Readability intent

The original Bash implementation was understandable because each capability read as one ordered block: inspect intent,
ensure prerequisites, inspect host state, perform the smallest mutation, and report the result. Preserve that reading
experience in the Rust structure where practical.

Do not introduce a generic workflow framework, plugin system, dynamic dispatch layer, or configurable dependency graph.
Use the smallest set of concrete functions and types needed to express the workflows below. Keep tightly related workflow
logic together, but retain a separate executor when an operation is genuinely shared by multiple capabilities.

### Safety constraints

The refactor must preserve these properties:

1. YAML cannot supply commands, shell fragments, plugins, or arbitrary executor behavior.
2. Unsupported platforms and invalid configuration fail before host mutation.
3. Privileged destinations remain constrained and important files remain atomically published.
4. Executors inspect current state and avoid unnecessary mutations.
5. External command output and downloaded metadata remain validated.
6. Prerequisites and manager bootstraps remain deduplicated.
7. Platform-specific behavior remains behind Linux or macOS workflow selection.
8. Existing unrelated worktree changes remain untouched.
9. `tests/cli.rs` remains one integration-test file; do not split it as part of this refactor.

### Completion criteria

The handoff is complete when:

1. Planner code visibly follows the nested capability model and the fixed orders in this document.
2. `init`, `check`, and standalone `dotfiles` visibly follow their proposed command workflows.
3. Linux and macOS `apply` produce the proposed workflow order.
4. Linux and macOS `update` produce the proposed workflow order.
5. Linux APT update performs only metadata refresh and the selected upgrade policy.
6. Moving YAML mappings does not change execution order.
7. Planner tests cover workflow ordering, derived prerequisites, and bootstrap deduplication.
8. Integration tests cover command workflows and the intentional Linux APT update behavior change.
9. Contributor documentation uses `capability workflow`, `operation`, `executor`, and `execution stage` consistently.
10. Superseded planner code, compatibility paths, terminology, tests, and documentation have been removed.
11. Generated configurations, formatting, Clippy, tests, rustdoc, shell checks, and release packaging all pass.

Configuration ownership and execution order are separate. A workflow can belong to `shared`, `os.linux`, or `os.macos`,
but its position in `cozydot.yaml` does not control when it runs. Cozydot executes workflows in the fixed order below and
skips workflows with no active intent.

A capability workflow can contain other capability workflows. Leaf workflows produce the typed operations that Cozydot
executes.

## Init

```text
1. Initialization workflow
   1.1. Resolve and validate the configuration root
   1.2. Select the embedded preset
   1.3. Synchronize the active configuration
   1.4. Synchronize bundled dotfiles
   1.5. Publish the managed-file manifest
```

## Check

```text
1. Configuration validation workflow
   1.1. Resolve the active configuration path
   1.2. Deserialize the complete configuration
   1.3. Validate host-independent configuration invariants
```

`check` is read-only and intentionally does not detect or validate the current platform.

## Linux dotfiles

```text
1. Dotfiles workflow
   1.1. Shared dotfiles workflow
   1.2. Linux dotfiles workflow
   1.3. Conflict handling workflow
```

## macOS dotfiles

```text
1. Dotfiles workflow
   1.1. Shared dotfiles workflow
   1.2. macOS dotfiles workflow
   1.3. Conflict handling workflow
```

The conflict workflow refuses unmanaged conflicts without changing dotfiles by default. With `--replace`, it backs up
all discovered conflicts before applying either platform's combined dotfile package list.

## Linux apply

```text
1. System workflow
   1.1. Administrative access
   1.2. Platform requirements
   1.3. Debian APT components
   1.4. Linux system state
   1.5. Derived system prerequisites

2. Linux package workflow
   2.1. APT workflow
        2.1.1. Direct packages
        2.1.2. Applicable third-party repositories
        2.1.3. Aggregated repository conflicts and packages
   2.2. Flatpak workflow

3. Shared tools workflow
   3.1. Rust workflow
   3.2. Go workflow
   3.3. Node.js workflow
   3.4. Python workflow

4. Shared package workflow
   4.1. Cargo workflow
   4.2. npm workflow

5. Linux binary workflow
   5.1. Deb package workflows
   5.2. AppImage workflows

6. Shared font workflow
   6.1. Nerd Fonts workflow

7. Dotfiles workflow
   7.1. Shared dotfiles
   7.2. Linux dotfiles

8. Integration workflow
   8.1. Docker workflow
   8.2. VirtualBox workflow
   8.3. VS Code workflow

9. Linux desktop workflow
   9.1. Theme workflow
   9.2. Terminal workflow
   9.3. Idle workflow
   9.4. GNOME workflow
```

## macOS apply

```text
1. System workflow
   1.1. Administrative access
   1.2. Xcode command line tools
   1.3. Rosetta

2. Homebrew workflow
   2.1. Homebrew availability
   2.2. Formulae
   2.3. Casks

3. Shared tools workflow
   3.1. Rust workflow
   3.2. Go workflow
   3.3. Node.js workflow
   3.4. Python workflow

4. Shared package workflow
   4.1. Cargo workflow
   4.2. npm workflow

5. Shared font workflow
   5.1. Nerd Fonts workflow

6. Dotfiles workflow
   6.1. Shared dotfiles
   6.2. macOS dotfiles

7. Integration workflow
   7.1. VS Code workflow

8. macOS desktop workflow
   8.1. Appearance workflow
   8.2. Dock workflow
   8.3. Finder workflow
   8.4. Keyboard workflow
   8.5. Trackpad workflow
```

## Linux update

```text
1. APT update workflow
   1.1. Refresh package metadata
   1.2. Standard or full system upgrade

2. Flatpak update workflow

3. Shared tool update workflow
   3.1. Rust workflow
   3.2. Go workflow
   3.3. Node.js workflow
   3.4. Python workflow

4. Shared package update workflow
   4.1. Cargo workflow
   4.2. npm workflow

5. Shared font update workflow
   5.1. Nerd Fonts workflow
```

The APT update workflow updates existing APT-managed state. Repository publication, configured package installation, and
repository conflict handling belong to `cozydot apply`.

## macOS update

```text
1. Homebrew update workflow
   1.1. Formulae
   1.2. Casks

2. Shared tool update workflow
   2.1. Rust workflow
   2.2. Go workflow
   2.3. Node.js workflow
   2.4. Python workflow

3. Shared package update workflow
   3.1. Cargo workflow
   3.2. npm workflow

4. Shared font update workflow
   4.1. Nerd Fonts workflow
```
