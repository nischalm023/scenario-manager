//! `simrunner` is Agent Studio's simulation runner (spec 08 section 3.5), which exists so that each run attempt runs
//! in a process of its own, a container or a Job's pod, on inputs it fetches itself at the plan's pins and with
//! results it delivers itself: every step before simtime through the WPS client and spec 07's crate, up to the
//! manifest (`attempt`); the runners' listener as the attempt reaches it, through which the runner reads its inputs,
//! its attempt's control, beats every 5 s of wall clock from the start, and once the run is over sends its recording to
//! the API to store and reports its ending and the two files (`door`); the run itself through dessim's harness, its
//! recording written as desvis headless writes one, and the metrics, series and ending derived from that recording
//! (`simulate`, `recording`); and the run's progress and its picture's frames streamed live to the streaming server
//! its job spec names, when it names one (`stream`). It is started as `simrunner --job <job.json> --out <folder>`, its token in
//! `STUDIO_RUNNER_TOKEN`. Attempt control carries a person's cancellation and SIGTERM the platform's ending of the run,
//! both into the runner's one stop (`stop`).

pub mod attempt;
pub mod door;
pub mod recording;
pub mod simulate;
pub mod stop;
pub mod stream;

use native_modules as _;
