//! `made_site`: a site made from a shell template in the world-proto-server `STUDIO_WPS_URL` names, for the browser
//! tests that open a made site's Map (brief I1.12), written scope by scope as a world-proto-server without changesets
//! across scopes takes them, since Create site's one changeset needs world-proto's phase 3 (spec 05 section 4.4). A root
//! that already holds a changeset is left as it is, so a test may ask again. With `--touch-flow`, one changeset lands
//! on the site's flow alone, as a flow edit would, for the test that follows a branch (spec 03 statement 10).
//!
//!   cargo run -p studio-api --example made_site -- <site root> <name> [shell, l-shape by default]
//!   cargo run -p studio-api --example made_site -- --touch-flow <site root>

#[path = "../tests/support/plan_site.rs"]
mod plan_site;

use std::env;
use std::process::ExitCode;
use std::sync::Arc;

use serde_json::json;
use studio_protocol::{DeclaredId, ScopePath, Seq};
use wps_client::draft::Op;
use wps_client::wire::WireChangeset;
use wps_client::wps::Wps;
use wps_client::{BlockingCompute, Client};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if let [flag, root] = &args[..]
        && flag == "--touch-flow"
    {
        return match touch_flow(root).await {
            Ok(seq) => {
                println!("{root}/flow moved to changeset {seq}");
                ExitCode::SUCCESS
            }
            Err(fault) => {
                eprintln!("made_site: {fault}");
                ExitCode::FAILURE
            }
        };
    }
    let [root, name, rest @ ..] = &args[..] else {
        eprintln!("made_site: name the site's root and its name, and optionally the shell");
        return ExitCode::FAILURE;
    };
    let shell = rest.first().map_or("l-shape", String::as_str);
    let Ok(path) = root.parse::<ScopePath>() else {
        eprintln!("made_site: `{root}` is not a scope path");
        return ExitCode::FAILURE;
    };
    let client = match Client::from_env(Arc::new(BlockingCompute)).await {
        Ok(client) => client,
        Err(fault) => {
            eprintln!("made_site: {fault}");
            return ExitCode::FAILURE;
        }
    };
    match client.tree().await.map(|tree| tree.head(&path)) {
        Ok(Some(head)) if head > Seq::new(0) => {
            println!("{root} is made already, at changeset {head}");
            return ExitCode::SUCCESS;
        }
        Ok(_) => {}
        Err(fault) => {
            eprintln!("made_site: {fault}");
            return ExitCode::FAILURE;
        }
    }
    let (made, seqs) = plan_site::made_site(&client, shell, &[], root, name).await;
    for (scope, seq) in seqs {
        println!("made {scope} at changeset {seq}");
    }
    println!("made {made} from template/site/{shell}");
    ExitCode::SUCCESS
}

/// Declare one more return rule in `root`'s flow, the flow's changeset moving alone.
async fn touch_flow(root: &str) -> Result<Seq, String> {
    let client = Client::from_env(Arc::new(BlockingCompute)).await.map_err(|fault| fault.to_string())?;
    let root: ScopePath = root.parse().map_err(|fault| format!("{fault}"))?;
    let flow = root.join(&"flow".parse().map_err(|fault| format!("{fault}"))?).map_err(|fault| format!("{fault}"))?;
    let base = client.tree().await.map_err(|fault| fault.to_string())?.head(&flow).ok_or("the site has no flow")?;
    let rule = json!({ "flow.return_rule": { "destination": "nearest_eligible", "min_backlog": 2 } });
    let components = rule.as_object().cloned().unwrap_or_default();
    let op = Op::Declare { id: DeclaredId::mint(), name: Some("Return rule, touched".into()), components };
    let changeset = WireChangeset::of_one_scope(flow, base, "tests", "A flow changeset alone (tests)", vec![op]);
    let landed = client.wps().post_changeset(&changeset).await.map_err(|fault| format!("{fault:?}"))?;
    landed.seq.ok_or_else(|| "the flow did not move".to_owned())
}
