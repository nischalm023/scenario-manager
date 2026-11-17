//! Within `wps-client`, `routes` is the one place a world-proto-server URL is built (spec 04 section 3.12): every
//! scope route is `/scopes/<path>/-/<operation>`, the one shape `STUDIO_WPS_ROUTES` names, `path`; the spelling before
//! WPS's scope paths answers 404 at any depth, so no other is built.

use std::str::FromStr;

use studio_protocol::ScopePath;
use url::Url;

/// The variable naming the route shape.
pub const ROUTES_VAR: &str = "STUDIO_WPS_ROUTES";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RouteShape {
    #[default]
    Path,
}

impl FromStr for RouteShape {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        match text {
            "path" => Ok(Self::Path),
            other => Err(format!(
                "{ROUTES_VAR}={other}: this client speaks only `path`, `/scopes/<path>/-/<operation>`, the shape \
                 world-proto-server has answered since its scope paths landed (c384e59)"
            )),
        }
    }
}

/// Where WPS answers, and every URL this client sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routes {
    base: Url,
    shape: RouteShape,
}

impl Routes {
    pub fn new(base: Url, shape: RouteShape) -> Result<Self, String> {
        if !matches!(base.scheme(), "http" | "https") || base.query().is_some() || base.cannot_be_a_base() {
            return Err(format!(
                "{base} is not an http or https address of world-proto-server, such as http://127.0.0.1:7413"
            ));
        }
        Ok(Self { base, shape })
    }

    pub fn base(&self) -> &Url {
        &self.base
    }

    pub fn shape(&self) -> RouteShape {
        self.shape
    }

    fn at(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        let prefix = self.base.path().trim_end_matches('/');
        url.set_path(&format!("{prefix}{path}"));
        url
    }

    pub fn healthz(&self) -> Url {
        self.at("/healthz")
    }

    pub fn protocol(&self) -> Url {
        self.at("/protocol")
    }

    pub fn protocol_defs(&self) -> Url {
        self.at("/protocol/defs")
    }

    pub fn scopes(&self) -> Url {
        self.at("/scopes")
    }

    /// `/scopes?under=<path>`: every scope at or below one path (scopes phase 3).
    pub fn scopes_under(&self, under: &ScopePath) -> Url {
        let mut url = self.at("/scopes");
        url.query_pairs_mut().append_pair("under", under.as_str());
        url
    }

    /// `/scopes/<path>/-/<operation>`.
    pub fn scope(&self, scope: &ScopePath, operation: &str) -> Url {
        match self.shape {
            RouteShape::Path => self.at(&format!("/scopes/{scope}/-/{operation}")),
        }
    }

    /// `/scopes/<path>/-/<operation>/<item>`, an entity id or a derivation address written whole.
    pub fn scope_item(&self, scope: &ScopePath, operation: &str, item: &str) -> Url {
        match self.shape {
            RouteShape::Path => self.at(&format!("/scopes/{scope}/-/{operation}/{item}")),
        }
    }

    pub fn registry(&self) -> Url {
        self.at("/registry")
    }

    pub fn registry_component(&self, component: &str) -> Url {
        self.at(&format!("/registry/{component}"))
    }

    pub fn generators(&self) -> Url {
        self.at("/registry/generators")
    }

    pub fn generator(&self, generator: &str) -> Url {
        self.at(&format!("/registry/generators/{generator}"))
    }
}
