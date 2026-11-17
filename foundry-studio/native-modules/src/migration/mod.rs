//! Within `native_modules`, `migration` holds files moved out of dessim as they were, changed only where their new
//! place forces a line (a path, the link to this crate): the tasks (`flows`), the core's tests that need a task to
//! drive them (`core_tests`), and `traffic_report`, a program that exports a recording's measurements to JSON. The
//! program is no module of this library: the manifest builds it as a binary. It left dessim with the tasks because it
//! reads a spec and a recording, which name tasks, so it must link them, and nothing in dessim may. The tasks and the
//! tests still name the core as `crate::…`, which the crate root's glob import of dessim resolves. A file leaves this
//! folder when it is given its home, named for its behaviour, and there names the core by its crate. No code is to be
//! written here; files only leave.

pub mod flows;

#[cfg(test)]
mod core_tests;
