//! Within `studio-streaming`, `floor` is what a browser draws an attempt on: the world document the attempt's
//! scenario block carries, compiled once per attempt into the floor and entity census a browser cannot compile, and
//! the scenario frame rewritten for a browser, as `studio/runs`' attach rewrites it. A recording leaves the floor and
//! census out (the simulator's file states the document alone, so the two cannot disagree); this server puts them in,
//! and takes out the run's data files and the producer's rebuild document, which nothing a browser draws reads.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use dessim::worldmap;
use rustc_hash::FxHashMap;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::spawn_blocking;
use worldview::{Frame, Opaque, Payload, Scenario, Tag};

use crate::streams::StreamKey;

/// One attempt's world compiled, in the form the wire carries.
#[derive(Debug, Clone)]
struct Scene {
    floor: Opaque,
    census: Opaque,
}

/// Every attempt's compiled world, kept until its stream is forgotten: a run's world never changes, and a compile
/// takes seconds on a site-sized document. One turn per attempt, so two first attachers compile it once.
#[derive(Default)]
pub struct Scenes {
    kept: Mutex<FxHashMap<StreamKey, Scene>>,
    turns: Mutex<FxHashMap<StreamKey, Arc<AsyncMutex<()>>>>,
}

impl Scenes {
    /// `scenario`, the attempt's scenario frame as runner and store hold it, as a browser receives it.
    pub async fn for_browser(&self, key: &StreamKey, scenario: &[u8]) -> Result<Bytes, String> {
        let (frame, _) = Frame::decode_at(scenario, 0).map_err(|fault| fault.to_string())?;
        let body = frame.contents(0).map_err(|fault| fault.to_string())?;
        let mut block = Scenario::from_bytes(&body, 0).map_err(|fault| fault.to_string())?;
        let scene = self.scene(key, &block).await?;
        block.data = None;
        block.producer = None;
        block.floor = Some(scene.floor);
        block.census = Some(scene.census);
        let body = block.to_bytes().map_err(|fault| fault.to_string())?;
        Frame::encode(Tag::Scenario, frame.header.t, &body, true).map(Bytes::from).map_err(|fault| fault.to_string())
    }

    /// The attempt's stream is forgotten: so is its world.
    pub fn forget(&self, key: &StreamKey) {
        self.kept.lock().expect("the scenes").remove(key);
        self.turns.lock().expect("the scene turns").remove(key);
    }

    async fn scene(&self, key: &StreamKey, block: &Scenario) -> Result<Scene, String> {
        let turn = self.turns.lock().expect("the scene turns").entry(key.clone()).or_default().clone();
        let _held = turn.lock().await;
        if let Some(scene) = self.kept.lock().expect("the scenes").get(key) {
            return Ok(scene.clone());
        }
        let world = block.world.0.clone();
        let scene = spawn_blocking(move || compile(&world)).await.map_err(|fault| fault.to_string())??;
        self.kept.lock().expect("the scenes").insert(key.clone(), scene.clone());
        Ok(scene)
    }
}

fn compile(world: &[u8]) -> Result<Scene, String> {
    let world = worldmap::load_from_bytes(world).map_err(|fault| format!("the world document will not load: {fault}"))?;
    let compiled = worldview_floor::compile(&world).map_err(|fault| format!("the document states no floor: {fault}"))?;
    Ok(Scene {
        floor: Opaque::of(&compiled.floor, "the compiled floor").map_err(|fault| fault.to_string())?,
        census: Opaque::of(&compiled.census, "the entity census").map_err(|fault| fault.to_string())?,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use studio_protocol::AttemptNumber;
    use worldview::{Frame, Payload, Scenario, Tag};

    use super::Scenes;
    use crate::streams::StreamKey;

    /// A real scenario frame: the first frame of the viewer's fixture recording, whose world compiles.
    pub(crate) fn scenario_frame() -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../viewer/tests/fixtures/minimal_easyloop_flow/recording.worldview");
        let bytes = std::fs::read(path).expect("the fixture recording");
        let (_, length) = Frame::decode_at(&bytes, 0).expect("its scenario frame");
        bytes[..length].to_vec()
    }

    /// The fixture's scenario frame stating another run: a different block, byte for byte, with the same world.
    pub(crate) fn scenario_frame_of_run(run: &str) -> Vec<u8> {
        let (frame, _) = Frame::decode_at(&scenario_frame(), 0).expect("the fixture's scenario frame");
        let mut block = Scenario::from_bytes(&frame.contents(0).unwrap(), 0).expect("its block");
        block.run = run.to_owned();
        Frame::encode(Tag::Scenario, frame.header.t, &block.to_bytes().unwrap(), true).expect("a frame encodes")
    }

    #[tokio::test]
    async fn a_scenario_frame_goes_to_a_browser_with_its_floor_and_without_the_data_files() {
        let key = StreamKey { run: "01KTJ153M7D109TG4G7BBVCZYW".parse().unwrap(), attempt: AttemptNumber(1) };
        let scenes = Scenes::default();
        let sent = scenes.for_browser(&key, &scenario_frame()).await.expect("the world compiles");
        let (frame, _) = Frame::decode_at(&sent, 0).expect("a frame");
        let block = Scenario::from_bytes(&frame.contents(0).unwrap(), 0).expect("a scenario block");
        assert!(block.floor.is_some() && block.census.is_some(), "the floor and census are put in");
        assert!(block.data.is_none() && block.producer.is_none(), "the data files and producer are taken out");
        assert_eq!(scenes.for_browser(&key, &scenario_frame()).await.unwrap(), sent, "the kept world, the same bytes");
    }
}
