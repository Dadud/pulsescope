#!/usr/bin/env python3
"""Run a command on the receiver laptop over SSH.

Password comes from DEPLOY_PASS so it never lands in argv, shell history, or
this file. Host/user default to the LAN appliance and can be overridden with
DEPLOY_HOST / DEPLOY_USER.

    DEPLOY_PASS=... python deploy/remote-exec.py 'docker ps'
    DEPLOY_PASS=... python deploy/remote-exec.py --put local.tar /remote/path
"""
import argparse
import os
import sys

import paramiko


def connect():
    host = os.environ.get("DEPLOY_HOST", "192.168.1.34")
    user = os.environ.get("DEPLOY_USER", "dadud")
    password = os.environ.get("DEPLOY_PASS")
    if not password:
        sys.exit("DEPLOY_PASS is not set")
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect(
        host,
        username=user,
        password=password,
        timeout=15,
        allow_agent=False,
        look_for_keys=False,
    )
    return client


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--put", nargs=2, metavar=("LOCAL", "REMOTE"))
    parser.add_argument("--timeout", type=float, default=None)
    parser.add_argument("command", nargs="?", default="")
    args = parser.parse_args()

    client = connect()
    try:
        if args.put:
            local, remote = args.put
            sftp = client.open_sftp()
            sftp.put(local, remote)
            sftp.close()
            print(f"PUT_OK {local} -> {remote}")
        if not args.command:
            return 0
        _, stdout, stderr = client.exec_command(args.command, timeout=args.timeout)
        for line in iter(stdout.readline, ""):
            sys.stdout.write(line)
            sys.stdout.flush()
        err = stderr.read().decode("utf-8", "replace")
        if err:
            sys.stderr.write(err)
        code = stdout.channel.recv_exit_status()
        print(f"REMOTE_EXIT={code}")
        return code
    finally:
        client.close()


if __name__ == "__main__":
    raise SystemExit(main())
