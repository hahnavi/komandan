# Test SSH Server

This directory contains a `Dockerfile` and a script to run a test SSH server in a Docker container.

## Usage

To build the Docker image and start the SSH server, run the following command:

```bash
./run_sshd.sh
```

The SSH server will be running on port 2222.

## Authentication

You can connect to the SSH server using either a password or public key authentication.

### Public Key Authentication

The `run_sshd.sh` script will automatically use your `~/.ssh/id_ed25519` public key. If the key does not exist, the script will generate it for you.

You can connect to the server using the following command:

```bash
ssh usertest@localhost -p 2222
```

### Password Authentication

The Docker container includes a test user with the following credentials:

- **Username:** `usertest`
- **Password:** `usertest`

The `usertest` user has `sudo` privileges.
