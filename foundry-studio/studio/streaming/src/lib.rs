//! `studio-streaming` is the streaming server (ruling D174): one process that takes a run's messages from its runner as
//! they happen, on an internal websocket port, and serves them to whoever watches, on an external one, replaying what
//! came before and then following live, so a run can be watched while it runs without its runner touching a database.
//! Several run at once; each registers itself in the API's database and says it is alive every 2 s of wall clock
//! (`registry`), so the API's dispatcher can point each new attempt at a live one, and each writes the top-level
//! figures of the runs it streams as they go (`stats`). Since live frames (26 October 2026) it also carries the run's
//! picture: each segment of worldview frames the runner locked, held in memory and on disk (`frames`), uploaded to
//! the run-object store as it arrives (`upload`), read from memory, disk or the store by one provider (`provider`) and
//! served to a viewer (`attach`) with the floor a browser cannot compile put into its scenario block (`floor`), the
//! disk copies swept by age and size (`cache`); and from the same readings each metric of the run, computed once for
//! every viewer of it (`metric_streams`). The parts: `config` is what the process is told, `streams` every stream it
//! holds in memory, within its bounds, and `listeners` its two ports and who may open a socket on them.

pub mod attach;
pub mod cache;
pub mod config;
pub mod floor;
pub mod frames;
pub mod listeners;
pub mod metric_streams;
pub mod provider;
pub mod registry;
pub mod stats;
pub mod streams;
pub mod upload;
