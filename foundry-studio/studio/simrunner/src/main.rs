use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use simrunner::attempt::{Handed, run};
use studio_protocol::{HOLD_VAR, TOKEN_VAR};
use tokio::runtime::{Builder, Runtime};

/// `simrunner --job <job.json> --out <folder>`: one attempt, simulated, what it left in the folder and delivered to
/// the API; exit 0 once that is delivered, whatever the ending says, and 1 when it could not be, so the platform's
/// view of the Job says the runner failed while what it left stays in the folder.
fn main() -> ExitCode {
    let handed = match handed() {
        Ok(handed) => handed,
        Err(words) => {
            eprintln!("simrunner: {words}");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(fault) => {
            eprintln!("simrunner: no runtime to run the attempt on: {fault}");
            return ExitCode::FAILURE;
        }
    };
    let delivered = runtime.block_on(run(&handed));
    leave(runtime);
    match delivered {
        Ok(_) => ExitCode::SUCCESS,
        Err(undelivered) => {
            eprintln!("simrunner: what the attempt left was not delivered: {undelivered}");
            ExitCode::FAILURE
        }
    }
}

/// The runtime let go without waiting for its blocking threads: a step of the preparation SIGTERM abandoned may still
/// compute on one, and the process goes once the attempt is delivered, inside the platform's grace.
fn leave(runtime: Runtime) {
    runtime.shutdown_background();
}

fn handed() -> Result<Handed, String> {
    let mut args = env::args().skip(1);
    let (mut job, mut out) = (None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--job" => job = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            other => return Err(format!("takes --job <file> and --out <folder>, not {other}")),
        }
    }
    let token = env::var(TOKEN_VAR).map_err(|_| format!("{TOKEN_VAR} holds no token"))?;
    let hold = match env::var(HOLD_VAR) {
        Ok(ms) => Some(Duration::from_millis(ms.trim().parse().map_err(|_| format!("{HOLD_VAR} is {ms:?}"))?)),
        Err(_) => None,
    };
    Ok(Handed {
        job: job.ok_or("--job names the job spec")?,
        out: out.ok_or("--out names the output folder")?,
        token,
        hold,
    })
}

#[cfg(test)]
mod tests {
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    use tokio::runtime::Builder;
    use tokio::task::spawn_blocking;

    use super::leave;

    /// A blocking step left computing, as SIGTERM leaves one of the preparation's, does not hold the runner's exit.
    #[test]
    fn the_runner_leaves_without_waiting_for_a_step_it_abandoned() {
        let runtime = Builder::new_multi_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            spawn_blocking(|| sleep(Duration::from_secs(30)));
        });
        let leaving = Instant::now();
        leave(runtime);
        assert!(leaving.elapsed() < Duration::from_secs(5), "left after {:?}", leaving.elapsed());
    }
}
