# AGENTS.md

## General principles
<!-- https://x.com/MarcosHernanz/status/2083954734487212511 -->
- Do not preserve backward compatibility. Remove obsolete paths instead of adding compatibility layers, fallbacks, or migrations.
- Choose the simplest implementation that fully meets the current requirements. Avoid speculative abstractions, configuration, and indirection.
- Grow the system in layers. Start from the smallest version that works end to end, and add each new capability on top of a product that already works. Never trade a working product for unfinished complexity.
- Keep components modular and concerns clearly separated.
- Prefer established, well-maintained libraries when they reduce overall complexity or improve reliability. Do not reimplement common functionality without a clear reason.
- Lean on the dependencies already in the project before writing your own implementation or adding packages. Do not assume a library lacks a capability without checking its documentation and types.
- Make architectural decisions for the long term. Do not accept a stopgap that only works for now and is meant to be replaced later.
- Study how established products solve the problem before designing a solution. Adopt their proven patterns and conventions rather than inventing an approach from scratch.

## Project conventions

- When adapting official documentation or an established implementation, preserve its structure and line order where practical. Make the smallest project-required diff, keep local terminology, cite the source, and explain only non-obvious deviations.

## Cozydot conventions

- Preserve upstream-documented names, values, paths, and identifiers so users can understand, troubleshoot, and maintain installed software from official documentation alone, without depending on Cozydot. Shorten names when no precision is lost.

## Code style

- Inline helpers used only once.
- Prefer chained combinators when they fit on one line. Use a direct `if` instead of wrapping the chain across lines.
- Extract long argument lists into variables so operation calls remain short.
- Keep assignments on one line. If that is too long, extract a meaningful intermediate variable instead of leaving `=` at the end of a line.
- Keep comments plain and directly useful.
- Avoid sentence case and trailing periods in comments.
- Use numerals instead of spelling out numbers.
- Prefer `&` / `/` over `and` / `or` in comments.
- Write comments in code-reading order, e.g. `// sort snaps so snapd, bare & core are last`.
- Prefer visually simple code over denser, more idiomatic code.

## Collaboration

- Treat questions, suggestions, reviews, and brainstorming as exploratory unless the user explicitly requests implementation.
- Briefly challenge incorrect, risky, or needlessly complex implementation details and recommend the simplest sound alternative. After the user makes an informed explicit choice, follow it unless it conflicts with safety, repository constraints, or feasibility.

## Supported platforms

Cozydot supports these Linux distributions:

- Debian
- Ubuntu
- Pop!_OS
- Linux Mint

Cozydot supports these Linux architectures:

- x86_64
- aarch64
- ARMv7 (32-bit)

Cozydot supports macOS on these architectures:

- Apple Silicon (arm64)
