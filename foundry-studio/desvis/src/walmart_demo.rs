//! Temporary Walmart demo defaults: resolve bundled inputs from the executable,
//! so Finder launches need no arguments and still wait for the user to press Run.

use std::path::Path;

use crate::GuiArgs;

pub fn configure(args: &mut GuiArgs) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|e| format!("cannot locate this app: {e}"))?;
    configure_at(args, &executable)
}

fn configure_at(args: &mut GuiArgs, executable: &Path) -> Result<(), String> {
    if args.map.is_some() || args.recording.is_some() {
        return Ok(());
    }
    let contents = executable.parent().and_then(Path::parent).ok_or("cannot locate app Contents directory")?;
    let spec = contents.join("Resources/walmart/walmart_dfw6_spec.json");
    if !spec.is_file() {
        return Err(format!(
            "bundled scenario is missing: {}. Extract the complete app from its ZIP before launching.",
            spec.display()
        ));
    }
    args.map = Some(spec);
    if args.label.is_none() {
        args.label = Some(format!("Walmart Demo {}", option_env!("WALMART_DEMO_VERSION").unwrap_or("development")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cli;
    use clap::Parser;

    #[test]
    fn bundle_defaults_are_portable_and_do_not_start_a_run() {
        let root = std::env::temp_dir().join(format!("walmart demo launch {}", std::process::id()));
        let contents = root.join("Walmart Simulation.app/Contents");
        let resources = contents.join("Resources/walmart");
        std::fs::create_dir_all(&resources).unwrap();
        let spec = resources.join("walmart_dfw6_spec.json");
        std::fs::write(&spec, "{}").unwrap();
        let mut args = Cli::parse_from(["desvis"]).gui;
        configure_at(&mut args, &contents.join("MacOS/desvis")).unwrap();
        assert_eq!(args.map.as_deref(), Some(spec.as_path()));
        assert!(!args.run && !args.test);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_inputs_do_not_require_a_bundle() {
        for arguments in [vec!["desvis", "another.json"], vec!["desvis", "--recording", "run.worldview"]] {
            let mut args = Cli::parse_from(arguments).gui;
            configure_at(&mut args, Path::new("/missing/Contents/MacOS/desvis")).unwrap();
        }
        let mut args = Cli::parse_from(["desvis"]).gui;
        assert!(configure_at(&mut args, Path::new("/missing/Contents/MacOS/desvis")).is_err());
    }
}
