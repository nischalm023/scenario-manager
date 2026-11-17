//! Within the Map Creator adapter, `format` is WHAT THE MAP CREATOR FORMAT IS
//! AND HOW ITS VALUES CONVERT: the export read as it is (`source`), the
//! disposition of every field it carries (`coverage`), the register of values no
//! export states (`params`), and the pass that rewrites millimetres as steps of
//! the site frame (`steps`). Import and export both stand on it. Nothing here
//! mints an identity.

pub mod coverage;
pub mod params;
pub mod source;
pub mod steps;
