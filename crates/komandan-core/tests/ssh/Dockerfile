FROM ubuntu:22.04

ARG PUBLIC_KEY

# Install SSH server and sudo
RUN apt-get update && apt-get install -y openssh-server sudo

# Create a new user and add to sudo group
RUN useradd -m -s /bin/bash usertest && \
    echo "usertest:usertest" | chpasswd && \
    adduser usertest sudo && \
    echo '%sudo ALL=(ALL) NOPASSWD:ALL' | tee /etc/sudoers.d/sudo-nopasswd

# Set up SSH for the new user
USER usertest
RUN mkdir /home/usertest/.ssh
RUN echo "$PUBLIC_KEY" >> /home/usertest/.ssh/authorized_keys
RUN chmod 700 /home/usertest/.ssh && \
    chmod 600 /home/usertest/.ssh/authorized_keys
USER root

# Expose SSH port
EXPOSE 22

# Start SSH server
RUN ssh-keygen -A && mkdir -p /run/sshd && chmod 755 /run/sshd

CMD ["/usr/sbin/sshd", "-D"]
