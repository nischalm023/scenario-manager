//! Within `tools`, `sites` are the agent's reads of the sites (spec 01 section 3.9; spec 04's reads; spec 03's find):
//! `site.list`, Home's list without its conversations; `site.read`, a site at a pin as the Map panel's summary reads
//! it, with each scope's changeset, its census by component set and the made-from record it was made by; and
//! `entity.find`, the map's search at a location. Each calls the route the panels call.

use agent_loop::{Class, Tool, ToolFuture};
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use wps_client::wps::Wps;

use super::{ToolContext, encoded, input, uri};
use crate::map::{find, summary};
use crate::sites::{SitesQuery, sites};

struct SiteList;

#[derive(Deserialize)]
struct ListInput {
    customer: Option<String>,
}

impl<S: Wps> Tool<ToolContext<S>> for SiteList {
    fn name(&self) -> &'static str {
        "site.list"
    }

    fn class(&self) -> Class {
        Class::Read
    }

    fn description(&self) -> &'static str {
        "List the sites world-proto-server holds, newest change first, optionally of one customer (its slug). Returns \
         { items: [{ site, name, branches: [{ name, heads: { scope: changeset } }], tags }] }."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "additionalProperties": false, "properties": { "customer": { "type": "string" } } })
    }

    fn describe(&self, input: &Value) -> String {
        match input["customer"].as_str() {
            Some(customer) => format!("Listing the sites of {customer}"),
            None => "Listing the sites".to_owned(),
        }
    }

    fn summarize(&self, _input: &Value, result: &Value) -> String {
        match result["items"].as_array().map_or(0, Vec::len) {
            1 => "Found 1 site".to_owned(),
            n => format!("Found {n} sites"),
        }
    }

    fn execute<'a>(&'a self, cx: &'a ToolContext<S>, given: Value) -> ToolFuture<'a, Value> {
        Box::pin(async move {
            let ListInput { customer } = input(&given)?;
            let listed = sites(State(cx.gate.clone()), uri("/api/sites")?, Ok(Query(SitesQuery { customer }))).await?.0;
            let items: Vec<Value> = (listed.items.iter())
                .map(|item| json!({ "site": item.site, "name": item.name, "branches": item.branches, "tags": item.tags }))
                .collect();
            Ok(json!({ "items": items }))
        })
    }
}

struct SiteRead;

impl<S: Wps> Tool<ToolContext<S>> for SiteRead {
    fn name(&self) -> &'static str {
        "site.read"
    }

    fn class(&self) -> Class {
        Class::Read
    }

    fn description(&self) -> &'static str {
        "Read one site at a pin (`customer/{c}/site/{s}@main`): the changeset each of its scopes stands at, how many \
         entities of each kind its map holds, and the template draft it was made from, if any. Returns { site, name, \
         seqs: { scope: changeset }, census: { kind: count }, made_from? }."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object", "additionalProperties": false, "required": ["site"],
            "properties": { "site": { "type": "string", "description": "the site's root scope and its pin, as customer/demo/site/lshape-6@main" } },
        })
    }

    fn describe(&self, input: &Value) -> String {
        format!("Reading the site {}", input["site"].as_str().unwrap_or("named"))
    }

    fn summarize(&self, _input: &Value, result: &Value) -> String {
        let name = result["name"].as_str().unwrap_or("the site");
        let kinds = result["census"].as_object().map_or(0, Map::len);
        format!("Read {name}: {kinds} kinds of entity on its map")
    }

    fn execute<'a>(&'a self, cx: &'a ToolContext<S>, given: Value) -> ToolFuture<'a, Value> {
        Box::pin(async move {
            let location = given["site"].as_str().unwrap_or_default().to_owned();
            let address = uri(&format!("/api/map/{location}"))?;
            let query = Query::try_from_uri(&address);
            let read = summary(State(cx.gate.clone()), address, Ok(Path(location)), query).await?.0;
            let seqs: Map<String, Value> =
                (read.resolved.iter()).map(|seen| (seen.scope.to_string(), json!(seen.seq))).collect();
            let census: Map<String, Value> =
                (read.counts.iter()).map(|row| (row.label.clone(), json!(row.count))).collect();
            let mut answer =
                json!({ "site": read.site.scope, "name": read.site.label, "seqs": seqs, "census": census });
            if let Some(made_from) = read.made_from {
                answer["made_from"] = json!(made_from);
            }
            Ok(answer)
        })
    }
}

struct EntityFind;

impl<S: Wps> Tool<ToolContext<S>> for EntityFind {
    fn name(&self) -> &'static str {
        "entity.find"
    }

    fn class(&self) -> Class {
        Class::Read
    }

    fn description(&self) -> &'static str {
        "Find entities on a site's map at a location (`customer/{c}/site/{s}/map@main`) whose name, kind or external id \
         holds the words `q`. Returns { items: [{ key, label, components, centre_mm, on_floor }], more }; refer to an \
         entity by its key."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object", "additionalProperties": false, "required": ["site", "q"],
            "properties": {
                "site": { "type": "string", "description": "a location of the site's map, as customer/demo/site/lshape-6/map@main" },
                "q": { "type": "string", "minLength": 1 },
                "limit": { "type": "integer", "minimum": 1, "maximum": 500 },
            },
        })
    }

    fn describe(&self, input: &Value) -> String {
        format!("Finding \"{}\" on the map", input["q"].as_str().unwrap_or_default())
    }

    fn summarize(&self, input: &Value, result: &Value) -> String {
        let found = result["items"].as_array().map_or(0, Vec::len);
        let more = if result["more"].as_bool() == Some(true) { " and more" } else { "" };
        format!("Found {found} entities{more} matching \"{}\"", input["q"].as_str().unwrap_or_default())
    }

    fn execute<'a>(&'a self, cx: &'a ToolContext<S>, given: Value) -> ToolFuture<'a, Value> {
        Box::pin(async move {
            let location = given["site"].as_str().unwrap_or_default().to_owned();
            let mut pairs = vec![("q", given["q"].as_str().unwrap_or_default().to_owned())];
            pairs.extend(given["limit"].as_u64().map(|limit| ("limit", limit.to_string())));
            let query: Vec<String> = pairs.iter().map(|(key, value)| format!("{key}={}", encoded(value))).collect();
            let address = uri(&format!("/api/find/{location}?{}", query.join("&")))?;
            let query = Query::try_from_uri(&address);
            let found = find(State(cx.gate.clone()), address, Ok(Path(location)), query).await?.0;
            Ok(json!(found))
        })
    }
}

pub fn tools<S: Wps>() -> Vec<Box<dyn Tool<ToolContext<S>>>> {
    vec![Box::new(SiteList), Box::new(SiteRead), Box::new(EntityFind)]
}
