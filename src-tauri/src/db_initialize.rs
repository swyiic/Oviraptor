// Each migration phase that `initialize` runs in order, split out so the launch
// sequence stays readable. Included from db.rs; every phase borrows the already
// opened connection and reports failures as `String`, exactly as the inline body did.
include!("db_initialize_columns.rs");
include!("db_initialize_repairs.rs");
include!("db_initialize_budgets.rs");
