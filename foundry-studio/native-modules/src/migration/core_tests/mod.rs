//! Within `migration`, `core_tests` holds unit tests of the core, dessim, that need a task to drive them: a relay's
//! pickup units under a navigation, its stock and custody under `VtmReturn` and `VtmForward`, and span parity's whole
//! runs. dessim's own unit tests link no task, so these moved here unchanged; each rejoins its subject when that
//! subject leaves dessim too.

mod relay;
mod span_parity;
