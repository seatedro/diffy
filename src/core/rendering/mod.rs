pub mod flat_rows;
pub mod layout_engine;
pub mod prepared_rows;

pub use flat_rows::{DiffRowType, FlatDiffRow, flatten_carbon_file_diff, flatten_file_diff};
pub use layout_engine::{DiffDisplayRow, DiffLayoutConfig, DiffLayoutEngine};
pub use prepared_rows::{PreparedRow, PreparedRowsCacheKey, prepare_rows};
