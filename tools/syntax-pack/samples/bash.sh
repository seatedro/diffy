#!/usr/bin/env bash
# Greets everyone named on the command line.
greet() {
  local name="$1"
  echo "hello, $name"
}
for who in "$@"; do
  greet "$who"
done
