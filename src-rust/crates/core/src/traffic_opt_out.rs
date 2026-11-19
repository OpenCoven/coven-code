//! Environment variables that turn off requests the app makes on its own.
//!
//! Each variable only has to be present: `0` and an empty value turn the
//! request off as well.

use std::ffi::OsString;

/// Turns off the requests of every other variable in this module, and the
/// official MCP registry lookup.
pub const DISABLE_NONESSENTIAL_TRAFFIC_ENV: &str = "COVEN_CODE_DISABLE_NONESSENTIAL_TRAFFIC";

/// Turns off the release check an interactive session makes at startup.
pub const DISABLE_UPDATE_CHECK_ENV: &str = "COVEN_CODE_DISABLE_UPDATE_CHECK";

/// Turns off the model catalog refresh from models.dev.
pub const DISABLE_MODELS_FETCH_ENV: &str = "COVEN_CODE_DISABLE_MODELS_FETCH";

/// Whether the request that `variable` turns off is turned off in this
/// process's environment, by `variable` itself or by
/// [`DISABLE_NONESSENTIAL_TRAFFIC_ENV`].
pub fn traffic_opted_out(variable: &str) -> bool {
    traffic_opted_out_in(variable, |name| std::env::var_os(name))
}

/// [`traffic_opted_out`] for the environment that `lookup` reads.
pub fn traffic_opted_out_in(variable: &str, lookup: impl Fn(&str) -> Option<OsString>) -> bool {
    lookup(variable).is_some() || lookup(DISABLE_NONESSENTIAL_TRAFFIC_ENV).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opted_out_with(variable: &str, set: &[(&str, &str)]) -> bool {
        traffic_opted_out_in(variable, |name| {
            set.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn nothing_set_leaves_every_request_on() {
        for variable in [DISABLE_UPDATE_CHECK_ENV, DISABLE_MODELS_FETCH_ENV] {
            assert!(!opted_out_with(variable, &[]), "{variable}");
        }
    }

    #[test]
    fn each_variable_turns_off_only_its_own_request() {
        for value in ["1", "0", ""] {
            let update = [(DISABLE_UPDATE_CHECK_ENV, value)];
            assert!(opted_out_with(DISABLE_UPDATE_CHECK_ENV, &update));
            assert!(!opted_out_with(DISABLE_MODELS_FETCH_ENV, &update));
            let models = [(DISABLE_MODELS_FETCH_ENV, value)];
            assert!(opted_out_with(DISABLE_MODELS_FETCH_ENV, &models));
            assert!(!opted_out_with(DISABLE_UPDATE_CHECK_ENV, &models));
        }
    }

    #[test]
    fn the_nonessential_traffic_variable_turns_off_the_update_check_and_the_models_fetch() {
        for value in ["1", "0", ""] {
            let set = [(DISABLE_NONESSENTIAL_TRAFFIC_ENV, value)];
            for variable in [DISABLE_UPDATE_CHECK_ENV, DISABLE_MODELS_FETCH_ENV] {
                assert!(opted_out_with(variable, &set), "{variable} with {value:?}");
            }
        }
    }
}
