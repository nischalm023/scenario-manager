//! Budgeted application projections over all four preserved site inputs.
mod fixtures;
use fixtures::documents;
use worldlens::{Budget, Library, Matcher, Object};
const LIBRARY: &str = foundry_world_lenses::LEGACY_WPS_LENS;
#[test]
fn a_budget_never_changes_an_answer() {
    let library = Library::parse(LIBRARY).expect("the shipped library");
    for fixture in documents() {
        let matcher = Matcher::new(&fixture.world.entities);
        let mut budget = Budget { steps: 20_000_000, bytes: 128 << 20 };
        for lens in library.lenses() {
            let left = budget.bytes;
            let within = matcher
                .project_within(lens, &mut budget)
                .unwrap_or_else(|over| panic!("{}: `{}` within the budget: {over}", fixture.name, lens.name()));
            assert_eq!(
                within,
                Matcher::new(&fixture.world.entities).project(lens),
                "{}: `{}`",
                fixture.name,
                lens.name()
            );
            assert_eq!(left - budget.bytes, written(&within), "{}: `{}`", fixture.name, lens.name());
        }
    }
}

fn written(objects: &[Object]) -> u64 {
    objects.iter().map(|object| object.to_json().to_string().len() as u64).sum()
}
