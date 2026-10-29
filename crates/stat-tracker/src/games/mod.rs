//! Game rows, filters and page totals as the desktop app shows them: the
//! GUI pages and the daemon's page totals compute them with the same code.
//! Pure functions over `storage` rows; nothing here touches the store.

pub mod aggregate;
pub mod model;
pub mod rows;
pub mod small_sample;
