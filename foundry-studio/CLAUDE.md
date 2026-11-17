# CLAUDE.md

## The line-by-line review process

This process exists so that every PR ships fully understood by the person who
owns it: the agent writes the code, and the user proves that understanding by
staging every change themselves — at once a checklist, a forced reading of
the code, and an opportunity for direct edits that keep real proficiency.

1. Every PR originates as a draft, and SHALL NOT be marked ready for review
   until the user has staged every line of its diff.
2. When the PR is deemed complete, the agent rebases the branch onto main
   and runs `git reset main`, leaving the branch's full diff unstaged in
   the working tree.
3. The user reviews the diff in their IDE and stages each line or hunk
   themselves; the agent never stages. VS Code's Source Control panel is
   recommended — it lists every file with unstaged changes. The GitHub PR
   diff viewer does not count as review.
4. Staging a hunk means "I understand this change and agree with it." The
   user asks the agent about anything unclear and stages nothing they do
   not understand; direct edits are welcome. Whatever stays unstaged gets
   reworked or discarded before finishing.
5. Once everything is staged, the agent commits the staged work and
   force-pushes; the PR may then be marked ready.

## The test gate

The gate is `cargo nextest run` at the root: the workspace's default members'
tests, every test its own process, the binaries' tests running concurrently
(`.config/nextest.toml`; install the runner once with
`cargo install cargo-nextest --locked`). It is meant to be run after every
change, so it has to stay a few minutes long: a test that runs a whole DFW6
or EasyLoop simulation, or a scale benchmark, carries
`#[cfg_attr(not(feature = "slow-gate"), ignore = "...")]` and runs in the slow
gate instead, `scripts/test-slow-gate.sh` (dessim's tests with its
`slow-gate` feature on), which the nightly workflow runs. Cut a new test's
simulated window to what its assertion needs before reaching for that
attribute. Dependencies build optimised in dev and test builds
(`[profile.dev.package."*"]` in the root manifest) because every DFW6 test
pays a document load; workspace crates do not, so an edit compiles as before.
Run one gate at a time on a machine: a few tests write fixed names in the
system's temp folder.

## Two clocks: scheduler time and simtime

Designs here go wrong when two orderings are confused. Think of a calendar
being filled in: next week is booked today, and the order of booking
(scheduler time) is not the order of the week (simtime). There is no
"present". A visit with no end date blocks booking the day after it until
its end is known, not until it is over; so a robot's path into a cell is
scheduled as soon as the robot ahead has its departure scheduled, and in
simtime both are inbound at once.

Every temporal word names its clock, in code, comments, designs and
explanations. "Is scheduled" and "is accepted" are scheduler time; "arrives",
"departs", "stands" and "waits" are simtime. "Now", "currently", "until it
leaves" and "is occupied" name neither.

## Module doc comments (charter rule)

The goal is that every module's meaning is OBVIOUS. Where the code and name
already make it obvious, no `//!` is owed. Where they don't, a touched file
gets a module doc comment (`//!`) stating: (1) what the module's noun MEANS
relative to its parent module, and (2) its specific purpose — plainly spoken,
no vague word-alignment with the filename ("integrates aisle snapshots into
capacity-time ratios per bucket/zone", never "utilization stuff"). If the
honest purpose is murky, overstuffed, or plural, write a
`TODO(overstuffed)`/`TODO(module)` in that doc calling for the rethink
instead of a vague blurb. The doc is a falsifiable charter: code that doesn't
match it is visibly misplaced — use it to drive relocation over time.

Comments are like salt: a little goes a long way and makes the dish tasty; a
lot makes it inedible. The ~15-comment-per-file budget (doc + inline) is the
default seasoning; a file that honestly cannot fit its soundness arguments in
15 is usually telling you it wants decomposing, not de-commenting — say so
with a `TODO(overstuffed)` rather than deleting load-bearing reasoning.

## Rust: prefer FxHashMap/FxHashSet

Default to `FxHashMap`/`FxHashSet` for new code:

```rust
use rustc_hash::{FxHashMap, FxHashSet};   // rustc-hash, already a dependency
```

Over `BTreeMap`/`BTreeSet`: the B-tree pays for its sorted iteration on every
insert and lookup; reach for it only when the code actually consumes that
ordering (ordered iteration, range queries). A justified B-tree gets a
`// ordered: <reason>` comment at the declaration.

Over `std::collections::HashMap`/`HashSet`: std's default hasher (SipHash)
buys DoS resistance we don't need — no map here is keyed by attacker-supplied
input — and FxHash is faster. Existing std maps don't need proactive
migration; switch them when already touching the code.

Caution: Fx map iteration order is arbitrary (it shifts with capacity and
insertion history) — never let it reach simulation results or user-visible
output. Where order matters, sort before iterating or use the B-tree.

## Rust: no inline qualified type paths

Import types with `use` instead of writing qualified paths inline
(`SimTime`, not `crate::model::SimTime`). Checker:
`scripts/find-qualified-paths.py --check` (exit 1 on violations, with fix
rules in the message). Enforced at commit time once per clone via:

```bash
git config core.hooksPath .githooks   # pre-commit gate
```

Claude Code also gets same-turn feedback via the PostToolUse hook in
`.claude/settings.json`. Exceptions the checker already knows: macro_rules!
bodies (must stay qualified — they resolve at expansion sites), idiomatic
families (`fmt::`/`io::` Result, `mpsc::`, `syn::Result`, egui/wgpu/eframe,
derive position), and lines waived with `// keep-qualified: <reason>`.
