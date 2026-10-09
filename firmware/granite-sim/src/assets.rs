//! The setup page, embedded. The simulator serves the uncompressed files
//! so the browser sees exactly what `firmware/web/` holds; the firmware
//! serves the gzipped copies from `firmware/web/dist/`.

/// One asset: path, content type, bytes.
pub struct Asset {
    /// Canonical request path.
    pub path: &'static str,
    /// Content type to send.
    pub content_type: &'static str,
    /// The file.
    pub bytes: &'static [u8],
}

/// Everything the page needs.
pub const ASSETS: &[Asset] = &[
    Asset {
        path: "/index.html",
        content_type: "text/html; charset=utf-8",
        bytes: include_bytes!("../../web/index.html"),
    },
    Asset {
        path: "/app.js",
        content_type: "text/javascript; charset=utf-8",
        bytes: include_bytes!("../../web/app.js"),
    },
    Asset {
        path: "/style.css",
        content_type: "text/css; charset=utf-8",
        bytes: include_bytes!("../../web/style.css"),
    },
];

/// Look an asset up by its canonical path.
pub fn asset(path: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|a| a.path == path)
}
