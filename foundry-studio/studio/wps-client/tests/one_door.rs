//! Spec 04 section 8's statement 9: one door. A source search finds `Uuid::new_v4` only behind `DeclaredId::mint`, no
//! WPS route or HTTP client outside this crate; every scope URL is `/scopes/<path>/-/<operation>`, built from the one
//! `STUDIO_WPS_ROUTES` shape; `ensure_scope` reuses an empty scope with the flag it asked for and answers
//! `ScopeExists` for one with the other flag or with live entities.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use studio_protocol::DeclaredId;
use support::{client, declare, path, write};
use url::Url;
use wps_client::routes::{RouteShape, Routes};
use wps_client::{WorldStore, WpsError};

fn sources(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).expect("a source folder").flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }
    found
}

/// Every Rust source of the three Agent Studio crates holding `needle`, as paths relative to the repository.
fn holding(needle: &str) -> Vec<String> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut holders = Vec::new();
    for crate_src in ["studio/wps-client/src", "studio/protocol/src", "studio/api/src"] {
        for file in sources(&repository.join(crate_src)) {
            if fs::read_to_string(&file).expect("a source").contains(needle) {
                let relative =
                    file.strip_prefix(&repository).expect("under the repository").to_string_lossy().into_owned();
                holders.push(relative.replace('\\', "/"));
            }
        }
    }
    holders.sort();
    holders
}

/// Statement 9: the one place an id is minted, the one HTTP client, the one place a scope route is spelled.
#[test]
fn there_is_one_door() {
    assert_eq!(holding("Uuid::new_v4"), vec!["studio/protocol/src/grammar/entity.rs"]);
    let entity =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../protocol/src/grammar/entity.rs"))
            .expect("a source");
    let minting = entity.find("pub fn mint()").expect("DeclaredId::mint");
    assert_eq!(entity.matches("Uuid::new_v4").count(), 1);
    assert!(entity.find("Uuid::new_v4").is_some_and(|at| at > minting && at - minting < 200), "inside `mint`");
    assert_eq!(holding("reqwest"), vec!["studio/wps-client/src/http.rs"]);
    assert_eq!(holding("\"/scopes"), vec!["studio/wps-client/src/routes.rs"]);
    assert_eq!(holding("/-/{"), vec!["studio/wps-client/src/routes.rs"], "a scope route is formatted in one place");
}

/// Statement 9: every scope URL is `/scopes/<path>/-/<operation>`, from the one shape `path`; another is refused.
#[test]
fn every_scope_url_has_one_shape() {
    let routes = Routes::new(Url::parse("http://127.0.0.1:7413").expect("a URL"), "path".parse().expect("path"))
        .expect("routes");
    let map = path("customer/demo/site/lshape-6/map");
    assert_eq!(
        routes.scope(&map, "export").as_str(),
        "http://127.0.0.1:7413/scopes/customer/demo/site/lshape-6/map/-/export"
    );
    let id = format!("{}/cell:3,17", DeclaredId::mint());
    assert_eq!(
        routes.scope_item(&map, "entities", &id).path(),
        format!("/scopes/customer/demo/site/lshape-6/map/-/entities/{id}")
    );
    assert!("segments".parse::<RouteShape>().is_err(), "the spelling before scope paths is not built");
}

/// Statement 9: `ensure_scope` reuses an empty scope with its flag, and answers `ScopeExists` for the other flag or
/// for a scope holding a live entity.
#[tokio::test]
async fn ensure_scope_reuses_only_an_empty_scope_of_its_flag() {
    let client = client().await;
    for prefix in ["customer", "customer/c", "customer/c/site"] {
        client.ensure_scope(&path(prefix), false).await.expect("a prefix");
    }
    let root = path("customer/c/site/s");
    let made = client.ensure_scope(&root, true).await.expect("created");
    assert_eq!(client.ensure_scope(&root, true).await.expect("reused"), made);
    assert!(matches!(client.ensure_scope(&root, false).await, Err(WpsError::ScopeExists { .. })));
    write(
        &client,
        &root,
        vec![declare(DeclaredId::mint(), json!({ "template.description": { "title": "t", "text": "x" } }))],
    )
    .await;
    assert!(
        matches!(client.ensure_scope(&root, true).await, Err(WpsError::ScopeExists { .. })),
        "a scope with a live entity"
    );
    let orphan = path("customer/none/site/x");
    assert!(
        matches!(client.ensure_scope(&orphan, true).await, Err(WpsError::Invalid { .. })),
        "a missing parent is our fault"
    );
}
