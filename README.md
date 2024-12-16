<div align="center">

<img alt="Komandan Logo" height="230" src="assets/komandan.png" />

# Komandan
#### Your army commander

[![Build Status]][github-actions] [![License:MIT]][license] [![Coverage]][codecov.io]

[Build Status]: https://github.com/hahnavi/komandan/actions/workflows/rust.yml/badge.svg
[github-actions]: https://github.com/hahnavi/komandan/actions
[License:MIT]: https://img.shields.io/badge/License-MIT-blue.svg
[license]: https://github.com/hahnavi/komandan/blob/main/LICENSE
[Coverage]: https://codecov.io/gh/hahnavi/komandan/branch/main/graph/badge.svg
[codecov.io]: https://app.codecov.io/gh/hahnavi/komandan

</div>

Komandan is a server automation tool that simplifies remote server management by leveraging the power and flexibility of the Lua programming language. It connects to target servers via SSH, following Ansible's agentless approach for streamlined operation. Komandan is designed to be easy to learn and use, even for those new to server automation.

> **Notice:** Komandan is in early development and currently supports Linux only. Updates will come as development progresses. Feedback is welcome—thank you for your support!

## Table of Contents
- [Installation](#installation)
- [Getting Started](#getting-started)
- [Usage](#usage)
- [`komando` function](#komando-function)
- [Modules](#modules)
- [Built-in functions](#built-in-functions)
- [Error Handling](#error-handling)
- [Contributing](#contributing)
- [License](#license)
- [Full Documentation](#full-documentation)

## Installation

Pre-built binaries for Komandan are available for Linux (x86_64 architecture only) on the [GitHub Releases](https://github.com/hahnavi/komandan/releases) page.

An installation script is provided for easy installation:

```bash
curl -fsSL https://raw.githubusercontent.com/hahnavi/komandan/main/install.sh | sh
```

This script will download the latest Komandan release for your system and install Komandan to `$HOME/.local/bin`.

## Getting Started

For comprehensive documentation, including detailed guides and references, please visit the [Komandan Documentation Site](https://komandan.vercel.app/docs).

## Usage

Here's a simple example to get you started. Create a Lua script named `main.lua`:

```lua
-- main.lua

local hosts = {
  {
    name = "webserver1",
    address = "10.20.30.41",
    tags = { "webserver" },
  },
  {
    name = "dbserver1",
    address = "10.20.30.42",
    tags = { "database" },
  },
}

komandan.set_defaults({
  user = "user1",
  private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
})

local webservers = komandan.filter_hosts(hosts, "webserver")

local task = {
  name = "Create a directory",
  komandan.modules.cmd({
    cmd = "mkdir -p /tmp/komandan_test",
  }),
}

for _, host in ipairs(webservers) do
  komandan.komando(host, task)
end
```

Run the script using the `komandan` command:

```sh
$ komandan main.lua
```

This script will connect to `webserver1` as `user1` using the specified SSH key and create the directory `/tmp/komandan_test`.

## `komando` function

The `komando` function is the core of Komandan. It executes tasks on remote hosts via SSH. It takes two arguments:

-   `host`: A table containing the connection details for the target server:
    -   `address`: The IP address or hostname.
    -   `port`: The SSH port (default: 22).
    -   `user`: The username for authentication.
    -   `private_key_file`: The path to the SSH private key file.
    -   `private_key_pass`: The passphrase for the private key (if encrypted).
    -   `password`: The password for authentication (if not using key-based auth).
-   `task`: A table defining the task to be executed:
    -   `name`: A descriptive name for the task (optional, used for logging).
    -   `module`: A table specifying the module to use and its arguments.
    -   `ignore_exit_code`: Whether to ignore non-zero exit codes (default: `false`).
    -   `elevate`: Whether to run the task with elevated privileges (default: `false`).
    -   `as_user`: The user to run the task as when elevated (optional).
    -   `env`: A table of environment variables to set for the task (optional).

The `komando` function returns a table with the following fields:

-   `stdout`: The standard output of the executed command or script.
-   `stderr`: The standard error output.
-   `exit_code`: The exit code of the command or script.

## Modules

Komandan provides built-in modules for common tasks, accessible through the `komandan.modules` table. Here's a quick overview of the available modules:

-   **`cmd`**: Execute shell commands on the remote host.
-   **`script`**: Run scripts on the remote host, either from a local file or provided directly.
-   **`upload`**: Upload files to the remote host.
-   **`download`**: Download files from the remote host.
-   **`apt`**: Manage packages on Debian/Ubuntu systems using `apt`.
-   **`lineinfile`**: Insert or replace lines in a file.
-   **`template`**: Render a jinja template file on the remote host.

For detailed explanations, arguments, and examples of each module, please refer to the [Modules section of the Komandan Documentation Site](https://komandan.vercel.app/docs/modules).

## Built-in functions

Komandan offers built-in functions to enhance scripting capabilities:

-   **`komandan.filter_hosts`**: Filters a list of hosts based on a pattern.
-   **`komandan.set_defaults`**: Sets default values for host connection parameters.

For detailed descriptions and usage examples of these functions, please visit the [Built-in Functions section of the Komandan Documentation Site](https://komandan.vercel.app/docs/functions/).

## Error Handling

Komandan provides error information through the return values of the `komando` function. If a task fails, the `exit_code` will be non-zero, and `stderr` may contain error messages. You can use the `ignore_exit_code` option in a task to continue execution even if a task fails.

Example:

```lua
local result = komandan.komando(host, task)

if result.exit_code ~= 0 then
  print("Task failed with exit code: " .. result.exit_code)
  print("Error output: " .. result.stderr)
else
  print("Task succeeded!")
  print("Output: " .. result.stdout)
end
```

## Contributing

Contributions to Komandan are welcome! If you'd like to contribute, please follow these guidelines:

1. Fork the repository on GitHub.
2. Create a new branch for your feature or bug fix.
3. Write your code and tests.
4. Ensure your code passes all existing tests.
5. Submit a pull request to the `main` branch.

Please report any issues or bugs on the [GitHub Issues](https://github.com/hahnavi/komandan/issues) page.

## License

Komandan is licensed under the [MIT License](LICENSE).

## Full Documentation

For more detailed information, examples, and advanced usage, please visit the [Komandan Documentation Site](https://komandan.vercel.app/docs).
