//! The application library declares its complete component/reference needs.
use worldlens::Library;
#[test]
fn each_shipped_lens_says_what_it_carries_and_what_it_follows() {
    let shipped = Library::parse(foundry_world_lenses::LEGACY_WPS_LENS).expect("the shipped library parses");
    let read: Vec<(String, Vec<String>, Vec<String>)> = shipped
        .lenses()
        .map(|lens| (lens.name().to_string(), lens.component_names(), lens.reference_fields()))
        .collect();
    let expected: [(&str, &[&str], &[&str]); 7] = [
        (
            "easyloop_station",
            &["cell", "easyloop", "frame", "pick", "port", "pose", "presentation_point", "region", "waiting"],
            &["frame.parent", "pose.frame"],
        ),
        ("lane", &["cell", "queue", "station", "tiling"], &["queue.path"]),
        (
            "rack_bay",
            &["cell", "pose", "rack", "relay", "slot", "stamp"],
            &["access_cell.cell", "relay.rack", "slot.rack"],
        ),
        ("placed_station", &["pose", "station"], &[]),
        ("pick_station", &["pose", "station"], &[]),
        ("floor_cohort", &["cohort", "robot_class"], &["cohort.robot_class"]),
        (
            "vtm_cohort",
            &["access", "cohort", "pose", "region", "robot_class", "sector", "stamp"],
            &["access.along", "access.rack", "cohort.assignment.to", "cohort.robot_class", "sector.members"],
        ),
    ];
    assert_eq!(read.len(), expected.len(), "the library ships seven lenses");
    for ((name, carries, follows), (want_name, want_carries, want_follows)) in read.iter().zip(expected) {
        assert_eq!(name, want_name);
        assert_eq!(carries, want_carries, "what `{name}` carries");
        assert_eq!(follows, want_follows, "what `{name}` follows");
        // Nothing a lens carries is spelled with a path on it, and no field is
        // shortened to the component it happens to sit on: that is the whole
        // difference between the two lists.
        for field in follows {
            assert!(!carries.contains(field), "`{name}` lists the field `{field}` as a component it carries");
        }
    }
}

#[test]
fn the_application_library_does_not_repeat_component_names_in_field_paths() {
    let library = Library::parse(foundry_world_lenses::LEGACY_WPS_LENS).unwrap();
    for pattern in library.patterns() {
        assert!(pattern.repeated_component_paths().is_empty(), "{}", pattern.name());
    }
}
