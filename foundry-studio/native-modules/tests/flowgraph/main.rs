//! The flow compiler's tests (spec 06 section 9): the two flows it is held to, the relay flow of section 6.1 on the
//! L-Shape made at 6 aisles and the store-replenishment litmus fixture of statement 4, and one test per problem code.

use native_modules as _;

mod determinism;
mod litmus;
mod modes;
mod parameters;
mod problems;
mod relay;
mod support;
