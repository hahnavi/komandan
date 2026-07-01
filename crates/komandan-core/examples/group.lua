local host = {
    name = "server1",
    address = "127.0.0.1",
    user = "usertest",
    connection = "ssh",
    host_key_check = false,
    private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
}

-- Example 1: Create a basic group
local create_group_task = {
    name = "Create basic group",
    komandan.modules.group({
        name = "developers",
        state = "present"
    }),
    elevate = true,
}

-- Example 2: Create a group with specific GID
local create_group_with_gid_task = {
    name = "Create group with specific GID",
    komandan.modules.group({
        name = "docker",
        gid = 1750,
        state = "present"
    }),
    elevate = true,
}

-- Example 3: Create a system group
local create_system_group_task = {
    name = "Create system group",
    komandan.modules.group({
        name = "webapp",
        system = true,
        state = "present"
    }),
    elevate = true,
}

-- Example 4: Create another group with specific GID
local create_another_group_task = {
    name = "Create another group with specific GID",
    komandan.modules.group({
        name = "restricted",
        gid = 2500,
        state = "present"
    }),
    elevate = true,
}

-- Example 5: Modify existing group GID
local modify_group_gid_task = {
    name = "Modify group GID",
    komandan.modules.group({
        name = "developers",
        gid = 2000,
        state = "present"
    }),
    elevate = true,
}

-- Example 6: Delete a group
local delete_group_task = {
    name = "Delete group",
    komandan.modules.group({
        name = "oldgroup",
        state = "absent"
    }),
    elevate = true,
}

-- Example 7: Force delete a group (even if it's a primary group)
local force_delete_group_task = {
    name = "Force delete group",
    komandan.modules.group({
        name = "problematic_group",
        state = "absent",
        force = true
    }),
    elevate = true,
}

-- Example 8: Create group with non-unique GID (advanced use case)
local create_non_unique_gid_task = {
    name = "Create group with non-unique GID",
    komandan.modules.group({
        name = "special_group",
        gid = 1000,  -- This GID might already exist
        non_unique = true,
        state = "present"
    }),
    elevate = true,
}

-- Example 9: Create local group (platform-specific)
local create_local_group_task = {
    name = "Create local group",
    komandan.modules.group({
        name = "localgroup",
        local_group = true,
        state = "present"
    }),
    elevate = true,
}

-- Execute tasks
print("Creating basic group...")
komandan.komando(create_group_task, host)

print("Creating group with specific GID...")
komandan.komando(create_group_with_gid_task, host)

print("Creating system group...")
komandan.komando(create_system_group_task, host)

print("Creating another group with specific GID...")
komandan.komando(create_another_group_task, host)

-- Uncomment to test other operations:
-- komandan.komando(modify_group_gid_task, host)
-- komandan.komando(delete_group_task, host)
-- komandan.komando(force_delete_group_task, host)
-- komandan.komando(create_non_unique_gid_task, host)
-- komandan.komando(create_local_group_task, host)

print("Group management tasks completed!")
