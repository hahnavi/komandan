use crate::checks::base::{
    CheckResult,
    result_validation::{StandardFields, create_validated_result, set_exists_field},
};
use std::collections::HashMap;

use super::{FileParameters, FileState};

/// Compare expected file parameters with actual file state
pub(super) fn compare_file_state(expected: &FileParameters, actual: &FileState) -> CheckResult {
    let mut actual_map = HashMap::new();
    let mut validation_passed = true;

    // Always include existence in actual state using standard field name
    set_exists_field(&mut actual_map, actual.exists);

    // Check existence expectation
    if let Some(expected_exists) = expected.exists
        && expected_exists != actual.exists
    {
        validation_passed = false;
    }

    // If file doesn't exist, we can't check other properties
    if !actual.exists {
        if expected.mode.is_some() || expected.owner.is_some() || expected.group.is_some() {
            validation_passed = false;
        }

        if let Some(error) = &actual.error {
            return CheckResult::error(error.clone());
        }

        return create_validated_result(validation_passed, &actual_map, None, "file")
            .unwrap_or_else(|_| CheckResult::failure(actual_map));
    }

    // File exists, check properties
    if let Some(error) = &actual.error {
        return CheckResult::error(error.clone());
    }

    // Check mode using standard field name. The Option must be compared
    // first: when expected is Some and actual is None (stat didn't return a
    // mode), that is a validation failure, not a silent pass.
    if let Some(ref expected_mode) = expected.mode {
        match actual.mode {
            Some(ref actual_mode) => {
                actual_map.insert(StandardFields::MODE.to_string(), actual_mode.clone());
                if expected_mode != actual_mode {
                    validation_passed = false;
                }
            }
            None => {
                validation_passed = false;
            }
        }
    } else if let Some(ref actual_mode) = actual.mode {
        actual_map.insert(StandardFields::MODE.to_string(), actual_mode.clone());
    }

    // Check owner using standard field name (see mode comment above).
    if let Some(ref expected_owner) = expected.owner {
        match actual.owner {
            Some(ref actual_owner) => {
                actual_map.insert(StandardFields::OWNER.to_string(), actual_owner.clone());
                if expected_owner != actual_owner {
                    validation_passed = false;
                }
            }
            None => {
                validation_passed = false;
            }
        }
    } else if let Some(ref actual_owner) = actual.owner {
        actual_map.insert(StandardFields::OWNER.to_string(), actual_owner.clone());
    }

    // Check group using standard field name (see mode comment above).
    if let Some(ref expected_group) = expected.group {
        match actual.group {
            Some(ref actual_group) => {
                actual_map.insert(StandardFields::GROUP.to_string(), actual_group.clone());
                if expected_group != actual_group {
                    validation_passed = false;
                }
            }
            None => {
                validation_passed = false;
            }
        }
    } else if let Some(ref actual_group) = actual.group {
        actual_map.insert(StandardFields::GROUP.to_string(), actual_group.clone());
    }

    create_validated_result(validation_passed, &actual_map, None, "file").unwrap_or_else(|_| {
        if validation_passed {
            CheckResult::success(actual_map)
        } else {
            CheckResult::failure(actual_map)
        }
    })
}
