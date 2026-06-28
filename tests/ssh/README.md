# Test SSH Server

This directory contains a `Dockerfile` and a script to run a test SSH server in a Docker container.

## Usage

To build the Docker image and start the SSH server, run the following command:

```bash
./run_sshd.sh
```

The SSH server will be running on port 52222.

## Test port override

The integration tests (`tests/ssh_integration.rs`, `tests/ssh_lua_integration.rs`)
connect to port `52222` by default. Override with `KOMANDAN_SSH_PORT`, e.g. when
running against a host sshd on port 22 (as the GitHub Actions workflows do):

```bash
KOMANDAN_SSH_PORT=22 cargo test --test ssh_integration
```

## Authentication

You can connect to the SSH server using either a password or public key authentication.

### Public Key Authentication

The `run_sshd.sh` script will automatically use your `~/.ssh/id_ed25519` public key. If the key does not exist, the script will generate it for you.

You can connect to the server using the following command:

```bash
ssh usertest@localhost -p 52222
```

### Password Authentication

The Docker container includes a test user with the following credentials:

- **Username:** `usertest`
- **Password:** `usertest`

The `usertest` user has `sudo` privileges.
