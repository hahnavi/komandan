#!/usr/bin/env komandan

-- Example demonstrating the file validation functionality
-- This script shows how to use komandan.check.file to validate file properties

print("=== Komandan File Check Example ===")

-- Create a test file for demonstration
local test_file = "/tmp/komandan_check_example.txt"
local host = {
    name = "local",
    address = "localhost",
    connection = "local"
}

-- First, create a test file
print("\n1. Creating test file...")
local create_result = komandan.komando({
    name = "Create test file",
    komandan.modules.file({
        path = test_file,
        content = "This is a test file for komandan check functionality\n",
        mode = "0644",
        owner = os.getenv("USER") or "root"
    })
}, host)

if create_result.exit_code == 0 then
    print("✓ Test file created successfully")
else
    print("✗ Failed to create test file")
    print("Error:", create_result.stderr)
    os.exit(1)
end

-- 2. Check if file exists
print("\n2. Checking file existence...")
local exists_check = komandan.check.file({
    path = test_file,
    exists = true
})

print("Result:", exists_check.ok and "✓ PASS" or "✗ FAIL")
print("Actual state:")
for key, value in pairs(exists_check.actual) do
    print("  " .. key .. ": " .. value)
end

-- 3. Check file mode
print("\n3. Checking file mode...")
local mode_check = komandan.check.file({
    path = test_file,
    mode = "0644"
})

print("Result:", mode_check.ok and "✓ PASS" or "✗ FAIL")
print("Expected mode: 0644")
print("Actual mode:", mode_check.actual.mode or "unknown")

-- 4. Check file owner
print("\n4. Checking file owner...")
local owner_check = komandan.check.file({
    path = test_file,
    owner = os.getenv("USER") or "root"
})

print("Result:", owner_check.ok and "✓ PASS" or "✗ FAIL")
print("Expected owner:", os.getenv("USER") or "root")
print("Actual owner:", owner_check.actual.owner or "unknown")

-- 5. Check multiple properties at once
print("\n5. Checking multiple properties...")
local multi_check = komandan.check.file({
    path = test_file,
    exists = true,
    mode = "0644",
    owner = os.getenv("USER") or "root"
})

print("Result:", multi_check.ok and "✓ PASS" or "✗ FAIL")
print("All properties match:", multi_check.ok and "Yes" or "No")

-- 6. Test with non-existent file
print("\n6. Checking non-existent file...")
local nonexistent_check = komandan.check.file({
    path = "/tmp/this_file_does_not_exist_12345",
    exists = false
})

print("Result:", nonexistent_check.ok and "✓ PASS" or "✗ FAIL")
print("File exists:", nonexistent_check.actual.exists)

-- 7. Test failure case (expecting wrong mode)
print("\n7. Testing failure case (wrong mode expectation)...")
local failure_check = komandan.check.file({
    path = test_file,
    mode = "0755"  -- Wrong mode expectation
})

print("Result:", failure_check.ok and "✓ PASS" or "✗ FAIL (expected)")
print("Expected mode: 0755")
print("Actual mode:", failure_check.actual.mode or "unknown")

-- 8. Using k.check alias
print("\n8. Using k.check alias...")
local alias_check = k.check.file({
    path = test_file,
    exists = true
})

print("Result:", alias_check.ok and "✓ PASS" or "✗ FAIL")
print("k.check alias works:", alias_check.ok and "Yes" or "No")

-- Clean up
print("\n9. Cleaning up...")
local cleanup_result = komandan.komando({
    name = "Remove test file",
    komandan.modules.cmd({
        cmd = "rm -f " .. test_file
    })
}, host)

if cleanup_result.exit_code == 0 then
    print("✓ Test file cleaned up")
else
    print("✗ Failed to clean up test file")
end

print("\n=== Example Complete ===")
print("The komandan.check.file function provides read-only validation")
print("of file properties without modifying the file system.")