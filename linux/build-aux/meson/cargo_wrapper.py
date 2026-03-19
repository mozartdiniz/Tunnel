#!/usr/bin/env python3
"""
Wrapper around `cargo build` that copies the compiled binary to the Meson
build directory. Called from src/meson.build via custom_target.

Usage: cargo_wrapper.py --manifest-path PATH --bin NAME [--release] OUTPUT
"""

import argparse
import os
import shutil
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--manifest-path', required=True)
    parser.add_argument('--bin', required=True, dest='bin_name')
    parser.add_argument('--target-dir')
    parser.add_argument('--target')
    parser.add_argument('--release', action='store_true')
    parser.add_argument('output')
    args, extra = parser.parse_known_args()

    cmd = ['cargo', 'build', '--manifest-path', args.manifest_path, '--bin', args.bin_name]
    if args.target_dir:
        cmd += ['--target-dir', args.target_dir]
    if args.target:
        cmd += ['--target', args.target]
    if args.release:
        cmd.append('--release')
    cmd.extend(extra)

    env = os.environ.copy()
    result = subprocess.run(cmd, env=env)
    if result.returncode != 0:
        sys.exit(result.returncode)

    profile = 'release' if args.release else 'debug'
    manifest_dir = os.path.dirname(os.path.abspath(args.manifest_path))
    target_dir = args.target_dir if args.target_dir else os.path.join(manifest_dir, 'target')

    # When cross-compiling, cargo places the binary under target/{triple}/{profile}/.
    if args.target:
        bin_path = os.path.join(target_dir, args.target, profile, args.bin_name)
    else:
        bin_path = os.path.join(target_dir, profile, args.bin_name)

    # Windows binaries have a .exe extension.
    if args.target and 'windows' in args.target:
        bin_path += '.exe'

    shutil.copy2(bin_path, args.output)


if __name__ == '__main__':
    main()
