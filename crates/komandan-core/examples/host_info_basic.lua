-- Basic host_info usage example
-- This example demonstrates gathering system information from a local host

local host = {
    name = "local-host",
    address = "localhost",
    connection = "local"
}

print("=== Basic Host Info Example ===")
print("Gathering system information from local host...")

-- Get host information
local info = komandan.host_info(host)

if info then
    print("\n--- Operating System Information ---")
    print("Distribution: " .. (info.os.distribution or "Unknown"))
    print("Version: " .. (info.os.version or "Unknown"))
    print("Family: " .. (info.os.family or "Unknown"))
    print("Kernel: " .. (info.os.kernel or "Unknown"))
    print("Hostname: " .. (info.os.hostname or "Unknown"))

    print("\n--- CPU Information ---")
    print("Model: " .. (info.cpu.model or "Unknown"))
    print("Cores: " .. (info.cpu.cores and tostring(info.cpu.cores) or "Unknown"))

    print("\n--- Memory Information ---")
    print("Total Memory: " .. (info.memory.total_mb and tostring(info.memory.total_mb) .. " MB" or "Unknown"))
    print("Free Memory: " .. (info.memory.free_mb and tostring(info.memory.free_mb) .. " MB" or "Unknown"))

    print("\n✓ Host information gathered successfully!")
else
    print("✗ Failed to gather host information")
end