#!/bin/sh
# Make git fetch the private world-proto repository over SSH, where CI and image builds authenticate with the
# read-only deploy key an SSH agent holds. Cargo's dependency URLs stay https, which a developer's own git
# credentials serve; this rewrite applies only where it runs. GitHub's host key is pinned rather than trusted on
# first use (https://api.github.com/meta).
set -eu
mkdir -p -m 0700 "$HOME/.ssh"
echo 'github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl' >> "$HOME/.ssh/known_hosts"
git config --global url."ssh://git@github.com/greyorange/world-proto".insteadOf "https://github.com/greyorange/world-proto"
