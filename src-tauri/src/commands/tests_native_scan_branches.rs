// Native branch tests share the command fixture scope, but each concern lives
// in its own file so new regressions do not grow a monolithic test module.
include!("tests_native_scan_branches/branch_status.rs");
include!("tests_native_scan_branches/timeline.rs");
include!("tests_native_scan_branches/timeline_pages.rs");
include!("tests_native_scan_branches/history_directives.rs");
