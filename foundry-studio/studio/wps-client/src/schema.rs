//! Within `wps-client`, `schema` is spec 04 section 3.10, the schema cache: per component its definition, its latest
//! published schema compiled exactly as WPS compiles it (JSON Schema 2020-12 with `GET /protocol/defs` folded in under
//! WPS's URN, `wp:server/server/src/schema_defs.rs:89-91`) and its reference annotations; and the generators the
//! registry runs, each a definition names, at its highest published version, which is the set a changeset states as
//! its precondition. A published version never changes, so none is compiled twice; which version is latest can. Every
//! fetch is merged into the snapshot current when it lands, under the write lock, so concurrent refreshes never
//! drop each other's entries.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use jsonschema::{Resource, Validator};
use rustc_hash::FxHashMap;
use semver::Version;
use serde_json::Value;
use studio_protocol::Seq;

use crate::error::{Problem, ProblemKind, WpsError};
use crate::value::GeneratorSet;
use crate::walker::Annotations;
use crate::wire::{RefAnnotation, WireDefinition};
use crate::wps::Wps;

/// The URN WPS publishes its schema definitions under, which registry schemas `$ref`.
pub const DEFS_URN: &str = "urn:world-graph-server:defs";

/// The components loaded at start (spec 04 section 3.10): the eleven Agent Studio writes and `external_ids`.
pub const PRELOAD: &[&str] = &[
    "template.knob",
    "template.knob_binding",
    "template.presence",
    "template.target_scope",
    "template.placement",
    "template.description",
    "flow.node",
    "flow.material_edge",
    "flow.agent_segment",
    "flow.return_rule",
    "successor",
    "external_ids",
];

/// One component as the cache holds it.
#[derive(Clone)]
pub struct ComponentEntry {
    pub definition: WireDefinition,
    /// `None` for a component defined with nothing published, which WPS refuses to validate against.
    pub latest: Option<Version>,
    pub refs: Vec<RefAnnotation>,
    validator: Option<Arc<Validator>>,
}

/// What spec 05's engine reads of the registry, at one moment: every component held and the generator set.
#[derive(Clone)]
pub struct RegistrySnapshot {
    pub proto_version: String,
    pub components: FxHashMap<String, ComponentEntry>,
    pub generators: GeneratorSet,
    /// The changeset of `registry/generators` the set was read at (`GET /registry/generators`'s `at`).
    pub generator_head: Seq,
}

impl Default for RegistrySnapshot {
    fn default() -> Self {
        Self {
            proto_version: String::new(),
            components: FxHashMap::default(),
            generators: GeneratorSet::default(),
            generator_head: Seq::new(0),
        }
    }
}

impl Annotations for RegistrySnapshot {
    fn refs(&self, component: &str) -> &[RefAnnotation] {
        self.components.get(component).map(|entry| entry.refs.as_slice()).unwrap_or_default()
    }
}

impl RegistrySnapshot {
    /// The names of `components` this snapshot holds no entry for, whose references no walk could find: a caller
    /// that must vouch for every reference stops on them rather than read them as holding none.
    pub fn missing<'a>(&self, components: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let mut missing: Vec<String> =
            components.into_iter().filter(|name| !self.components.contains_key(*name)).map(str::to_owned).collect();
        missing.sort();
        missing.dedup();
        missing
    }

    /// Whether `component`'s definition names a generator, so writing it on an entity moves what that entity derives.
    pub fn is_generator_input(&self, component: &str) -> bool {
        self.components.get(component).is_some_and(|entry| entry.definition.generator.is_some())
    }

    /// Validate one payload as WPS's gate would: `Unregistered` for a name with no published schema.
    pub fn validate(&self, component: &str, payload: &Value) -> Result<(), Box<Problem>> {
        let entry = self.components.get(component);
        let Some((version, validator)) =
            entry.and_then(|entry| Some((entry.latest.as_ref()?, entry.validator.as_ref()?)))
        else {
            let text = format!("component `{component}` has no published schema in the registry");
            return Err(Box::new(Problem::new(ProblemKind::Unregistered, text)));
        };
        validator.validate(payload).map_err(|fault| {
            let text = format!("component `{component}` does not satisfy `{component}@{version}`: {fault}");
            Box::new(Problem::new(ProblemKind::SchemaInvalid, text))
        })
    }
}

/// The cache: a snapshot swapped whole when anything moves, and every compiled version kept.
#[derive(Default)]
pub struct SchemaCache {
    current: RwLock<Arc<RegistrySnapshot>>,
    defs: Mutex<Option<Resource>>,
    compiled: Mutex<FxHashMap<(String, String), Arc<Validator>>>,
    compilations: AtomicUsize,
}

impl SchemaCache {
    pub fn snapshot(&self) -> Arc<RegistrySnapshot> {
        self.current.read().expect("the schema cache's lock").clone()
    }

    /// How many schema versions this cache has compiled, each once.
    pub fn compilations(&self) -> usize {
        self.compilations.load(Ordering::Relaxed)
    }

    fn merge(&self, change: impl FnOnce(&mut RegistrySnapshot)) {
        let mut current = self.current.write().expect("the schema cache's lock");
        let mut fresh = (**current).clone();
        change(&mut fresh);
        *current = Arc::new(fresh);
    }

    /// Load the definitions document, `names`, and the generator set: what `Client::start` calls.
    pub async fn load<S: Wps>(&self, wps: &S, proto_version: &str, names: &[&str]) -> Result<(), WpsError> {
        let defs = wps.protocol_defs().await?;
        let resource = Resource::from_contents(defs).map_err(|fault| WpsError::Invalid {
            text: format!("GET /protocol/defs is not a schema document this validator reads: {fault}"),
        })?;
        *self.defs.lock().expect("the schema cache's lock") = Some(resource);
        let mut fetched = Vec::new();
        for name in names {
            if let Some(entry) = self.fetch(wps, name).await? {
                fetched.push(((*name).to_owned(), entry));
            }
        }
        let (generators, head) = generator_set(wps).await?;
        self.merge(|snapshot| {
            snapshot.proto_version = proto_version.to_owned();
            snapshot.components.extend(fetched);
            (snapshot.generators, snapshot.generator_head) = (generators, head);
        });
        Ok(())
    }

    /// The entry for `name`, fetched on first use.
    pub async fn ensure<S: Wps>(&self, wps: &S, name: &str) -> Result<Option<ComponentEntry>, WpsError> {
        if let Some(entry) = self.snapshot().components.get(name) {
            return Ok(Some(entry.clone()));
        }
        self.refresh(wps, name).await
    }

    /// Fetch `name` again, as after a refusal naming a version other than the cached latest, or an unregistered or
    /// unpublished component; versions already compiled are kept.
    pub async fn refresh<S: Wps>(&self, wps: &S, name: &str) -> Result<Option<ComponentEntry>, WpsError> {
        let entry = self.fetch(wps, name).await?;
        self.merge(|snapshot| {
            match &entry {
                Some(entry) => snapshot.components.insert(name.to_owned(), entry.clone()),
                None => snapshot.components.remove(name),
            };
        });
        Ok(entry)
    }

    /// Whether a refusal naming `component@version` means the cache is behind, in which case it refreshes it.
    pub async fn note_refusal<S: Wps>(&self, wps: &S, component: &str, version: &str) -> Result<bool, WpsError> {
        let held = self.snapshot().components.get(component).and_then(|entry| entry.latest.clone());
        if held.is_some_and(|held| held.to_string() == version) {
            return Ok(false);
        }
        self.refresh(wps, component).await?;
        Ok(true)
    }

    /// The pass every 60 s of wall clock: every held component fetched again, and the generator family's head read.
    pub async fn revalidate<S: Wps>(&self, wps: &S) -> Result<(), WpsError> {
        let names: Vec<String> = self.snapshot().components.keys().cloned().collect();
        for name in names {
            self.refresh(wps, &name).await?;
        }
        let (generators, head) = generator_set(wps).await?;
        self.merge(|snapshot| (snapshot.generators, snapshot.generator_head) = (generators, head));
        Ok(())
    }

    async fn fetch<S: Wps>(&self, wps: &S, name: &str) -> Result<Option<ComponentEntry>, WpsError> {
        let Some(component) = wps.component(name).await? else { return Ok(None) };
        let latest = component
            .versions
            .iter()
            .filter_map(|version| Some((Version::parse(&version.version).ok()?, version)))
            .max_by(|a, b| a.0.cmp(&b.0));
        let Some((version, published)) = latest else {
            return Ok(Some(ComponentEntry {
                definition: component.component,
                latest: None,
                refs: Vec::new(),
                validator: None,
            }));
        };
        let validator = self.compiled(name, &version, &published.schema)?;
        Ok(Some(ComponentEntry {
            definition: component.component.clone(),
            latest: Some(version),
            refs: published.refs.clone(),
            validator: Some(validator),
        }))
    }

    fn compiled(&self, name: &str, version: &Version, schema: &Value) -> Result<Arc<Validator>, WpsError> {
        let key = (name.to_owned(), version.to_string());
        let mut compiled = self.compiled.lock().expect("the schema cache's lock");
        if let Some(validator) = compiled.get(&key) {
            return Ok(validator.clone());
        }
        let defs = self.defs.lock().expect("the schema cache's lock").clone();
        let mut options = jsonschema::options();
        if let Some(defs) = defs {
            options.with_resource(DEFS_URN, defs);
        }
        let validator = options.build(schema).map_err(|fault| WpsError::Invalid {
            text: format!("`{name}@{version}` is published, and does not compile here: {fault}"),
        })?;
        self.compilations.fetch_add(1, Ordering::Relaxed);
        let validator = Arc::new(validator);
        compiled.insert(key, validator.clone());
        Ok(validator)
    }
}

/// The generators WPS runs, each with its highest published version and digest, and the family's head: only those a
/// component definition names, since a generator no definition names never runs and a changeset stating it is
/// refused (`wp:server/server/src/storage/generation.rs:163-247` at 092d695).
async fn generator_set<S: Wps>(wps: &S) -> Result<(GeneratorSet, Seq), WpsError> {
    let family = wps.generators().await?;
    let named: Vec<String> =
        wps.components().await?.into_iter().filter_map(|definition| definition.generator).collect();
    // ordered: a generator set compares and prints in name order.
    let mut set = BTreeMap::new();
    for definition in &family.generators {
        let Some(name) = definition.get("generator").and_then(Value::as_str) else { continue };
        if !named.iter().any(|named| named == name) {
            continue;
        }
        let published = wps.generator(name).await?;
        let highest = published
            .versions
            .iter()
            .filter_map(|version| Some((Version::parse(&version.version).ok()?, version.digest.clone())))
            .max_by(|a, b| a.0.cmp(&b.0));
        if let Some(highest) = highest {
            set.insert(name.to_owned(), highest);
        }
    }
    Ok((GeneratorSet(set), Seq::new(family.at)))
}
