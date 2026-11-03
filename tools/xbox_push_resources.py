#!/usr/bin/env python3
"""Copy prepared resources to an Xbox in Developer Mode, over Device Portal.

Horizon's package deliberately ships without Minecraft's files. This script
uploads the output of `horizon-prepare` into the app's LocalState folder,
which is the only place a UWP app may read them from.

It talks to Device Portal's REST API, the same one its file explorer uses, so
it needs nothing on the console beyond remote access being turned on.

    python3 tools/xbox_push_resources.py --host 192.168.1.50 --user me --password secret

Device Portal is HTTPS on port 11443 with a self-signed certificate, so
verification is off by default; pass --ca-bundle to check it against your own
copy of the certificate instead.
"""

from __future__ import annotations

import argparse
import os
import ssl
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path

# The app's package family name. It follows from the manifest's Identity Name
# and the publisher hash, so it changes if either does; --package overrides it.
DEFAULT_PACKAGE = "dev.horizonlab.Horizon_8wekyb3d8bbwe"
DEFAULT_PORT = 11443
# Where the game looks. Must match horizon-platform-uwp's Paths::resources_dir.
REMOTE_SUBDIR = "resources"


def build_opener(args: argparse.Namespace) -> urllib.request.OpenerDirector:
    if args.ca_bundle:
        context = ssl.create_default_context(cafile=args.ca_bundle)
    else:
        # Device Portal's certificate is self-signed and per-console, so there
        # is nothing to chain it to. The connection is still encrypted; it is
        # the identity that is unverified, which is why this must stay on a
        # network you trust.
        context = ssl._create_unverified_context()

    handlers: list[urllib.request.BaseHandler] = [
        urllib.request.HTTPSHandler(context=context)
    ]
    if args.user:
        manager = urllib.request.HTTPPasswordMgrWithDefaultRealm()
        root = f"https://{args.host}:{args.port}"
        manager.add_password(None, root, args.user, args.password or "")
        handlers.append(urllib.request.HTTPBasicAuthHandler(manager))
        handlers.append(urllib.request.HTTPDigestAuthHandler(manager))
    return urllib.request.build_opener(*handlers)


def upload(
    opener: urllib.request.OpenerDirector,
    args: argparse.Namespace,
    local: Path,
    remote_dir: str,
) -> None:
    """PUTs one file into LocalState/<remote_dir> as multipart/form-data."""
    query = urllib.parse.urlencode(
        {
            "knownfolderid": "LocalAppData",
            "packagefullname": args.package,
            "path": "\\LocalState\\" + remote_dir.replace("/", "\\"),
        }
    )
    url = f"https://{args.host}:{args.port}/api/filesystem/apps/file?{query}"

    boundary = uuid.uuid4().hex
    payload = bytearray()
    payload += f"--{boundary}\r\n".encode()
    payload += (
        f'Content-Disposition: form-data; name="{local.name}"; '
        f'filename="{local.name}"\r\n'.encode()
    )
    payload += b"Content-Type: application/octet-stream\r\n\r\n"
    payload += local.read_bytes()
    payload += f"\r\n--{boundary}--\r\n".encode()

    request = urllib.request.Request(url, data=bytes(payload), method="POST")
    request.add_header("Content-Type", f"multipart/form-data; boundary={boundary}")
    with opener.open(request, timeout=args.timeout):
        pass


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--host", required=True, help="the console's IP address")
    parser.add_argument("--port", type=int, default=DEFAULT_PORT)
    parser.add_argument("--user", help="Device Portal username")
    parser.add_argument("--password", help="Device Portal password")
    parser.add_argument(
        "--resources",
        type=Path,
        default=Path(".local/horizon-client/26.3"),
        help="the folder horizon-prepare wrote (default: %(default)s)",
    )
    parser.add_argument("--package", default=DEFAULT_PACKAGE)
    parser.add_argument("--ca-bundle", help="verify Device Portal against this certificate")
    parser.add_argument("--timeout", type=float, default=120.0)
    parser.add_argument(
        "--dry-run", action="store_true", help="list what would be sent, send nothing"
    )
    args = parser.parse_args()

    root: Path = args.resources
    if not root.is_dir():
        parser.error(
            f"{root} is not a folder. Run horizon-prepare first:\n"
            "  cargo run --release -p horizon-prepare -- --prepare "
            "--server-jar <path to server.jar>"
        )

    files = sorted(p for p in root.rglob("*") if p.is_file())
    if not files:
        parser.error(f"{root} is empty")

    total = sum(p.stat().st_size for p in files)
    print(f"{len(files)} files, {total / 1e6:.1f} MB from {root}")
    print(f"to {args.host}:{args.port} -> LocalState\\{REMOTE_SUBDIR}")

    if args.dry_run:
        for path in files[:20]:
            print("  ", path.relative_to(root))
        if len(files) > 20:
            print(f"   ... and {len(files) - 20} more")
        return 0

    if not args.user:
        print("warning: no --user; Device Portal usually requires authentication",
              file=sys.stderr)

    opener = build_opener(args)
    sent = 0
    for index, path in enumerate(files, 1):
        relative = path.relative_to(root)
        remote_dir = REMOTE_SUBDIR
        if relative.parent != Path("."):
            remote_dir = f"{REMOTE_SUBDIR}/{relative.parent.as_posix()}"
        try:
            upload(opener, args, path, remote_dir)
        except urllib.error.HTTPError as e:
            print(f"\n{relative}: HTTP {e.code} {e.reason}", file=sys.stderr)
            if e.code in (401, 403):
                print("Check --user and --password.", file=sys.stderr)
            return 1
        except urllib.error.URLError as e:
            print(f"\n{relative}: {e.reason}", file=sys.stderr)
            print(
                "Is the console in Developer Mode with remote access on?",
                file=sys.stderr,
            )
            return 1
        sent += 1
        print(f"\r  {index}/{len(files)}  {relative}"[:100].ljust(100), end="", flush=True)

    print(f"\rSent {sent} files.".ljust(100))
    print("Launch Horizon on the console; it should find the resources now.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
