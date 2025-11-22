#!/bin/bash

set -e

cd "$(dirname "$0")"

IMAGE_NAME="komandan-ssh-server"
CONTAINER_NAME="komandan-ssh-server"
SSH_KEY_PATH="$HOME/.ssh/id_ed25519"

# Generate SSH key if it doesn't exist
if [ ! -f "$SSH_KEY_PATH" ]; then
    echo "SSH key not found, generating a new one..."
    ssh-keygen -t ed25519 -f "$SSH_KEY_PATH" -N ""
fi

PUBLIC_KEY=$(cat "${SSH_KEY_PATH}.pub")

# Build the Docker image
docker build --build-arg PUBLIC_KEY="$PUBLIC_KEY" -t $IMAGE_NAME .

# Stop and remove existing container
if [ "$(docker ps -a -q -f name=$CONTAINER_NAME)" ]; then
    docker stop $CONTAINER_NAME
    docker rm $CONTAINER_NAME
fi

# Run the Docker container
docker run -d --rm --name $CONTAINER_NAME -p 22:22 $IMAGE_NAME

echo "SSH server is running on port 22"
