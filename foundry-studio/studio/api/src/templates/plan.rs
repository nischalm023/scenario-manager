//! Within `templates`, a `plan` is what an evaluation would write into a made site: per made scope (`.`, `map`,
//! `flow`, `fleet`) its declarations, each with its id, source key, label and components, and its records, kept as
//! JSON in the run-object store beside the floor (spec 05 section 3.7), so Create site writes exactly what was
//! evaluated and drawn; and the site's one changeset that plan makes, which the evaluation checks and Create site
//! sends.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use site_template::{Evaluation, Record};
use studio_protocol::{DeclaredId, EntityId, SitePath};
use wps_client::draft::Op;
use wps_client::schema::RegistrySnapshot;
use wps_client::site::SitePlan;
use wps_client::{Author, DocId, GeneratorSet, Prologue, WpsError};

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanFile {
    pub doc: String,
    pub primary_frame: Option<EntityId>,
    pub scopes: Vec<PlanScope>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanScope {
    /// The made scope relative to the site's root: `.`, `map`, `flow` or `fleet`.
    pub scope: String,
    pub declarations: Vec<PlanDeclaration>,
    pub records: Vec<PlanRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanDeclaration {
    pub id: DeclaredId,
    pub key: String,
    pub name: Option<String>,
    pub components: Map<String, Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanRecord {
    pub target: EntityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<Map<String, Value>>,
}

/// The plan of `evaluation`, its scopes in path order.
pub fn plan_of(evaluation: &Evaluation) -> Vec<u8> {
    let mut scopes: Vec<_> = evaluation.plan.iter().collect();
    scopes.sort_by(|a, b| a.0.cmp(b.0));
    let file = PlanFile {
        doc: evaluation.prologue.doc.to_string(),
        primary_frame: evaluation.prologue.primary_frame.clone(),
        scopes: scopes
            .into_iter()
            .map(|(scope, plan)| PlanScope {
                scope: scope.to_string(),
                declarations: plan
                    .declarations
                    .iter()
                    .map(|made| PlanDeclaration {
                        id: made.id,
                        key: made.key.to_string(),
                        name: made.name.clone(),
                        components: made.components.clone(),
                    })
                    .collect(),
                records: plan
                    .records
                    .iter()
                    .map(|record| match record {
                        Record::Override { target, components } => {
                            PlanRecord { target: target.clone(), components: Some(components.clone()) }
                        }
                        Record::Suppress { target } => PlanRecord { target: target.clone(), components: None },
                    })
                    .collect(),
            })
            .collect(),
    };
    serde_json::to_vec(&file).expect("a plan is JSON values, which always serialize")
}

impl PlanFile {
    /// The generator set the site's changeset states: the registry's when an entity it writes carries a component
    /// naming a generator, which WPS then holds the changeset to, and none otherwise, since WPS refuses a set stated
    /// where nothing derives (world-proto 092d695).
    pub fn generators(&self, registry: &RegistrySnapshot) -> GeneratorSet {
        let declared = self.scopes.iter().flat_map(|scope| &scope.declarations);
        let derives =
            declared.flat_map(|made| made.components.keys()).any(|component| registry.is_generator_input(component));
        if derives { registry.generators.clone() } else { GeneratorSet::default() }
    }

    /// The site's one changeset at `site` as the evaluation planned it: the root states the evaluation's document
    /// under `name`, its primary frame the made frame and no dependency (spec 05 section 3.6); each made entity is
    /// declared under the id the evaluation gave it and each record kept; `generators` is its precondition; and
    /// `operation` names it, so that every sending of it sends the same body.
    pub fn site_plan(
        self,
        site: &SitePath,
        name: &str,
        author: Author,
        message: String,
        operation: &str,
        generators: GeneratorSet,
    ) -> Result<SitePlan, WpsError> {
        let invalid = |text: String| WpsError::Invalid { text };
        let root = site.as_scope().clone();
        let prologue = Prologue {
            doc: self.doc.parse::<DocId>().map_err(|fault| invalid(format!("the plan's document id: {fault}")))?,
            kind: Some("world".to_owned()),
            name: Some(name.to_owned()),
            primary_frame: self.primary_frame,
            depends_on: Vec::new(),
        };
        let mut plan = SitePlan::new(root.clone(), prologue, author, message, operation)?;
        for made in self.scopes {
            let scope = match made.scope.as_str() {
                "." => root.clone(),
                below => {
                    below.parse().and_then(|below| root.join(&below)).map_err(|fault| invalid(fault.to_string()))?
                }
            };
            for declared in made.declarations {
                plan.declare(&scope, declared.id, declared.name, declared.components)?;
            }
            for record in made.records {
                let op = match record.components {
                    Some(components) => Op::Override { target: record.target, components },
                    None => Op::Suppress { target: record.target },
                };
                plan.record(&scope, op)?;
            }
        }
        plan.expect_generators(generators);
        Ok(plan)
    }
}
