//! The tracker's version, as this build reports it to the site and in
//! problem reports.
//!
//! Both read `CARGO_PKG_VERSION` of the package this file is built in:
//! stat-tracker-core, whose version is kept equal to the release version in
//! `crates/stat-tracker/Cargo.toml`. `the_version_constants_are_this_release`
//! in scuffed-stat-tracker and `dist/check-tag-version.sh` fail when the two
//! differ. They used to sit in `report::bundle` and `sync`, which made
//! `perf_log` and `packs` depend on those modules for one string each. They
//! are re-exported at those old paths.

/// `app_version` in report bundles and `perf.log`: the tracker version of
/// this build.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// `User-Agent` on every request the tracker makes to the site (uploads,
/// daemon config, reader packs), so the server can tell which tracker
/// version sent it (S5-SRV-4).
pub const USER_AGENT: &str = concat!("scuffed-stat-tracker/", env!("CARGO_PKG_VERSION"));
