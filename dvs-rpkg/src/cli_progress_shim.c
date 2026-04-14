// Thin C wrappers around cli's static inline progress bar API.
// cli/progress.h is header-only; these shims make the functions
// available as regular symbols that Rust can link to directly.
#include <Rinternals.h>
#include <cli/progress.h>

SEXP cli_progress_bar_shim(double total, SEXP config) {
    return cli_progress_bar(total, config);
}

void cli_progress_add_shim(SEXP bar, double inc) {
    cli_progress_add(bar, inc);
}

void cli_progress_set_shim(SEXP bar, double set) {
    cli_progress_set(bar, set);
}

void cli_progress_done_shim(SEXP bar) {
    cli_progress_done(bar);
}

void cli_progress_set_clear_shim(SEXP bar, int clear) {
    cli_progress_set_clear(bar, clear);
}
