//! Frontend files embedded into the binary at compile time, so the editor is a
//! single self-contained executable that never loads anything from the network.

/// One embedded file.
#[derive(Debug)]
pub struct Asset {
    /// URL path relative to `/static/` (or `index.html` for the page itself).
    pub path: &'static str,
    pub content_type: &'static str,
    pub body: &'static [u8],
}

macro_rules! asset {
    ($path:literal, $content_type:literal) => {
        Asset {
            path: $path,
            content_type: $content_type,
            body: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/", $path)),
        }
    };
}

/// The application page.
pub static INDEX: Asset = asset!("index.html", "text/html; charset=utf-8");

/// Files served under `/static/`.
pub static STATIC_ASSETS: &[Asset] = &[
    asset!("css/app.css", "text/css; charset=utf-8"),
    asset!("js/app.js", "text/javascript; charset=utf-8"),
    asset!("js/api.js", "text/javascript; charset=utf-8"),
    asset!("js/status.js", "text/javascript; charset=utf-8"),
    asset!("js/documents.js", "text/javascript; charset=utf-8"),
    asset!("js/files.js", "text/javascript; charset=utf-8"),
    asset!("js/editor/containers.js", "text/javascript; charset=utf-8"),
    asset!("js/editor/content.js", "text/javascript; charset=utf-8"),
    asset!("js/editor/dom.js", "text/javascript; charset=utf-8"),
    asset!("js/editor/editor.js", "text/javascript; charset=utf-8"),
    asset!("js/editor/insert.js", "text/javascript; charset=utf-8"),
    asset!("js/editor/render.js", "text/javascript; charset=utf-8"),
    asset!("js/editor/tables.js", "text/javascript; charset=utf-8"),
    asset!("js/editor/toolbar.js", "text/javascript; charset=utf-8"),
    asset!("favicon.svg", "image/svg+xml"),
];

/// Looks up a static asset by its path below `/static/`.
pub fn find(path: &str) -> Option<&'static Asset> {
    STATIC_ASSETS.iter().find(|asset| asset.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_known_assets() {
        let css = find("css/app.css").expect("stylesheet embedded");
        assert!(css.content_type.starts_with("text/css"));
        assert!(!css.body.is_empty());
    }

    #[test]
    fn rejects_unknown_and_traversal_paths() {
        for path in ["", "index.html", "../Cargo.toml", "css/../js/app.js", "js/missing.js", "/css/app.css"] {
            assert!(find(path).is_none(), "{path}");
        }
    }

    #[test]
    fn every_asset_is_non_empty_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for asset in STATIC_ASSETS {
            assert!(!asset.body.is_empty(), "{} is empty", asset.path);
            assert!(seen.insert(asset.path), "{} listed twice", asset.path);
        }
        assert!(INDEX.body.starts_with(b"<!doctype html>"));
    }
}
