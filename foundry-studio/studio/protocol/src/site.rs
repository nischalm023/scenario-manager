//! Within `studio-protocol`, `site` is a site as Home's list and the context bar read it: `GET /api/sites`'s answer
//! (spec 01 section 3.6), whose items are spec 01 section 3.9's `site.list` items with when each site last changed
//! and its live conversation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::clock::WallClock;
use crate::conversation::WaitingOn;
use crate::grammar::{BranchName, ScopePath, Seq, SitePath, TagName, Ulid};

/// One branch of a site and each of its scopes' heads on it; only `main` until world-proto-server has branches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "site.ts")]
pub struct SiteBranch {
    pub name: BranchName,
    // ordered: a site's scopes are listed in path order.
    pub heads: BTreeMap<ScopePath, Seq>,
}

/// A site's live conversation as the list shows it: what its open turn waits on, and whether a turn is working,
/// both null until the agent loop keeps spec 01 section 3.3's `turns` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "site.ts")]
pub struct SiteConversation {
    pub id: Ulid,
    pub waiting: Option<WaitingOn>,
    pub working: Option<bool>,
}

/// One site: its root scope; its name, from the root's prologue, null when the prologue states none; its branches;
/// its tags, none until world-proto-server has tags; `changed_at`, the newest `applied_at` among its scopes' heads
/// (wall clock); and its live conversation, null when nobody has opened one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "site.ts")]
pub struct SiteItem {
    pub site: SitePath,
    pub name: Option<String>,
    pub branches: Vec<SiteBranch>,
    pub tags: Vec<TagName>,
    pub changed_at: WallClock,
    pub conversation: Option<SiteConversation>,
}

/// `GET /api/sites?customer=`'s answer: every site whose root has landed a changeset, newest change first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "site.ts")]
pub struct SiteList {
    pub items: Vec<SiteItem>,
}
