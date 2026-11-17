//! Shared world-document loading for the CLI utilities (`info`, `route`,
//! `fpi`, `--export-png`): one sniff, one loader, one error message. A path
//! is either a spec file (`world` reference) or a world file `worldmap` reads,
//! a site's flagged export or a world-protocol document, as its `WorldFile`
//! tells them apart; the retired v3 `SimSpec` gets a conversion pointer
//! instead of a parse attempt.

use std::path::Path;

use dessim::scenario_gen::Racking;
use dessim::worldmap::{WorldFile, WorldMap};
use dessim::{ScenarioConfig, SimConfig};
use serde_json::Value;

/// Everything a CLI utility needs from a map argument.
pub(crate) struct CliWorld {
    pub world: WorldMap,
    pub scenario: ScenarioConfig,
    pub settings: SimConfig,
}

impl CliWorld {
    pub fn racking(&self) -> Racking {
        self.world.racking.clone()
    }
}

/// Load a world document or spec by path, or die with a conversion pointer.
pub(crate) fn load(map_path: &str) -> CliWorld {
    let path = Path::new(map_path);
    let json = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {map_path}: {e}"));
    let head: Value = serde_json::from_str(&json).unwrap_or(Value::Null);
    if head.get("world").is_some() && head.get("version").is_none() {
        let loaded =
            dessim::world_spec::load(path).unwrap_or_else(|e| panic!("failed to load world spec {map_path}: {e}"));
        return CliWorld { world: loaded.world, scenario: loaded.scenario, settings: loaded.settings };
    }
    let read = WorldFile::of(path, &head).unwrap_or_else(|e| panic!("failed to load world document {map_path}: {e}"));
    if read.is_some() {
        let world = dessim::worldmap::load_parsed(path, &head)
            .unwrap_or_else(|e| panic!("failed to load world document {map_path}: {e}"));
        let mut settings = SimConfig::default();
        return CliWorld { world, scenario: ScenarioConfig::default(), settings };
    }
    panic!(
        "{map_path} is not a world-protocol document or spec (version={:?}). The v3 design \
         format is retired; maps come in through WPS and its adapters.",
        head.get("version").and_then(Value::as_u64),
    );
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process;

    use super::load;

    /// A file written for one test, removed however the test ends.
    struct Written(PathBuf);

    impl Drop for Written {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    /// A file carrying a flagged export's `scope` beside an authored document's prologue is refused by name, the way
    /// `worldmap` refuses it, rather than taken for either.
    #[test]
    #[should_panic(expected = "carries `scope`, the keys of a site's flagged export")]
    fn a_file_marked_both_flagged_and_authored_is_refused() {
        let written = Written(env::temp_dir().join(format!("desvis-marked-both-{}.json", process::id())));
        let doc = r#"{"proto_version": "0.7.0", "doc": {"id": "d0000000-0000-4000-8000-000000000001"}, "scope": "s"}"#;
        fs::write(&written.0, doc).expect("written");
        load(written.0.to_str().expect("a UTF-8 path"));
    }
}
