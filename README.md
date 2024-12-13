<div align="center">

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

Komandan is a server automation tool that uses Lua programming language interface. It connects to target servers via SSH, following Ansible's approach for its simplicity and agentless operation on managed servers.

> **Notice:** Komandan is in early development and currently supports Linux only. Updates will come as development progresses. Feedback is welcome—thank you for your support!

## Table of Contents
- [Usage](#usage)
- [`komando` function](#komando-function)
- [Modules](#modules)
  - [`cmd` module](#cmd-module)
  - [`script` module](#script-module)
  - [`upload` module](#upload-module)
  - [`download` module](#download-module)
  - [`apt` module](#apt-module)
- [Built-in functions](#built-in-functions)
  - [`komandan.filter_hosts`](#komandan-filter-hosts)
  - [`komandan.set_defaults`](#komandan-set-defaults)

## Usage

Create a lua script:
```lua
-- main.lua

local host = {
  address = "10.20.30.40",
  user = "user1",
  private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
}

local task = {
    name = "Create a directory",
    komandan.modules.cmd({
        cmd = "mkdir /tmp/newdir"
    })
}

komandan.komando(host, task)
```

Run the script:
```sh
$ komandan main.lua
```

## `komando` function

Komandan has `komando` function that takes two arguments:
- `host`: a table that contains the following fields:
  - `address`: the IP address or hostname of the target server.
  - `port`: the SSH port to use for the connection (default is 22).
  - `user`: the username to use for authentication.
  - `private_key_file`: the path to the private key file for authentication.
  - `private_key_pass`: the passphrase for the private key file for authentication.
  - `password`: the password to use for authentication if no private key is provided.
- `task`: a table that contains the following fields:
  - `name`: a string that describes the task. It is used for logging purposes. (optional)
  - `module`: a table that contains the module to be executed and its arguments.
  - `ignore_exit_code`: a boolean that indicates whether to ignore the exit code of the task. If `true`, the script will continue even if the task returns a non-zero exit code. (default is `false`)
  - `elevate`: a boolean that indicates whether to run the task as root. (default is `false`)
  - `as_user`: a string that specifies the user to run the task as. Requires `elevate` to be `true`. (optional)

This function will execute the module on the target server and return the results:
- `stdout`: a string that contains the standard output of the module.
- `stderr`: a string that contains the standard error output of the module.
- `exit_code`: an integer that contains the exit code of the module.

## Modules

Komandan has several built-in modules that can be used to perform various tasks on the target server. These modules are located in the `komandan.modules` table.
### `cmd` module

The `cmd` module allows you to execute a shell command on the target server. It takes the following arguments:
- `cmd`: a string that contains the shell command to be executed.

### `script` module

The `script` module allows you to execute a script on the target server. It takes the following arguments:
- `script`: a string that contains the script to be executed.
- `from_file`: a string that contains the local path to the script file to be executed on the target server. (`script` and `from_file` parameters are mutually exclusive)
- `interpreter`: a string that specifies the interpreter to use for the script. If not specified, the script will be executed using the default shell.

### `upload` module

The `upload` module allows you to upload a file to the target server. It takes the following arguments:
- `src`: a string that contains the path to the file to be uploaded.
- `dst`: a string that contains the path to the destination file on the target server.

### `download` module

The `download` module allows you to download a file from the target server. It takes the following arguments:
- `src`: a string that contains the path to the file to be downloaded.
- `dst`: a string that contains the path to the destination file on the local machine.

### `apt` module

The `apt` module allows you to install packages on the target server. It takes the following arguments:
- `package`: a string that contains the name of the package to be installed.
- `action`: a string that specifies the action to be taken on the package. (default is `install`. Supported actions: `install`, `remove`, `purge`, `upgrade`, `autoremove`)
- `update_cache`: a boolean that indicates whether to update the package cache before installing the package. (default is `false`)
- `install_recommends`: a boolean that indicates whether to install recommended packages. (default is `true`)

## Built-in functions

Komandan provides several built-in functions that can be used to help write scripts.

### `komandan.filter_hosts`

The `filter_hosts` function takes two arguments:
- `hosts`: a table that contains the hosts to filter.
- `pattern`: a string that contains the name or tag to filter the hosts. It can be a regular expression by adding `~` at the beginning of the pattern.


The function returns a table that contains the filtered hosts.

Example:

```lua
local hosts = {
  {
    name = "server1",
    address = "10.20.30.41",
    tags = { "webserver", "database" },
  },
  {
    name = "server2",
    address = "10.20.30.42",
    tags = { "webserver" },
  },
  {
    name = "server3",
    address = "10.20.30.43",
    tags = { "database" },
  },
}

local filtered_hosts = komandan.filter_hosts(hosts, "webserver")
```

This will return the table `filtered_hosts` that contains only the hosts that have the name or tag `webserver`.

### `komandan.set_defaults`

The `set_defaults` function takes one argument:
- `data`: a table that contains the defaults to set.

Example:
```lua
komandan.set_defaults({
  user = "user1",
  private_key_file = os.getenv("HOME") .. "/id_ed25519",
})
```

Those defaults will be used by `komando` function when the host table doesn't contain the specified field.
