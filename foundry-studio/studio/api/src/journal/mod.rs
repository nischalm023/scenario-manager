//! Within `studio-api`, `journal` is a conversation's record and how it reaches the tabs (spec 01 section 3.5): the
//! writer, which appends an entry inside its caller's transaction; the in-process hub, which wakes the
//! conversation's streams once that transaction commits and carries the items the journal never keeps; the stream
//! itself; the readers that turn rows back into entries; and the folds that turn entries into the state of what they
//! name.

mod hub;
mod projection;
mod read;
mod stream;
mod turns;
mod writer;

pub use hub::{Hub, HubItem, HubRecvError, LiveRefused, SUBSCRIBER_BUFFER, Subscription};
pub use projection::{FOLDED_KINDS, Fold, Named, readable};
pub use read::{entries_after, entries_of_kinds};
pub use stream::{PING_EVERY, stream};
pub use turns::{TURN_KINDS, TurnFold, open_turn};
pub use writer::{Appended, JournalError, NewEntry, Written, append, append_to_site, held};
