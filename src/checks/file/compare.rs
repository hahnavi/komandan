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

    // Check mode using standard field name
    if let Some(ref actual_mode) = actual.mode {
        actual_map.insert(StandardFields::MODE.to_string(), actual_mode.clone());

        if let Some(ref expected_mode) = expected.mode
            && expected_mode != actual_mode
        {
            validation_passed = false;
        }
    }

    // Check owner using standard field name
    if let Some(ref actual_owner) = actual.owner {
        actual_map.insert(StandardFields::OWNER.to_string(), actual_owner.clone());

        if let Some(ref expected_owner) = expected.owner
            && expected_owner != actual_owner
        {
            validation_passed = false;
        }
    }

    // Check group using standard field name
    if let Some(ref actual_group) = actual.group {
        actual_map.insert(StandardFields::GROUP.to_string(), actual_group.clone());

        if let Some(ref expected_group) = expected.group
            && expected_group != actual_group
        {
            validation_passed = false;
        }
    }

    create_validated_result(validation_passed, &actual_map, None, "file").unwrap_or_else(|_| {
        if validation_passed {
            CheckResult::success(actual_map)
        } else {
            CheckResult::failure(actual_map)
        }
    })
}
