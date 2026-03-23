pub mod flat_rows;
pub mod layout_engine;
pub mod prepared_rows;

pub use flat_rows::{flatten_file_diff, DiffRowType, FlatDiffRow};
pub use layout_engine::{DiffDisplayRow, DiffLayoutConfig, DiffLayoutEngine};
pub use prepared_rows::{prepare_rows, PreparedRow, PreparedRowsCacheKey};
