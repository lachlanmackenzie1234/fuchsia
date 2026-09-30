# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Command-line interface for driver-lab host operations.

Structured JSON results go to stdout; diagnostics go to stderr. Exit
categories follow the host specification: 0 for success, 2 for local
argument, plan, or permission errors; `run` returns the run's exit
category, 7 when evidence cannot be persisted, and 4 when interrupted.
"""

from __future__ import annotations

import argparse
import asyncio
import dataclasses
import datetime
import json
import sys
from collections.abc import Sequence
from pathlib import Path

from driver_lab.api import EXIT_EVIDENCE, EXIT_OPERATION, DriverLab
from driver_lab.consent import ConsentDecision
from driver_lab.evidence import EvidenceError
from driver_lab.models import AccessClass, AccessRequest, Decision, ReadGrant
from driver_lab.permissions import (
    GrantStoreError,
    add_grant,
    load_grants,
    resolve,
    revoke_grant,
)
from driver_lab.plans import PlanError, plan_digest, validate_plan
from driver_lab.transport import ProxyTransport

EXIT_SUCCESS = 0
EXIT_ERROR = 2


def _grant_row(grant: ReadGrant) -> dict[str, object]:
    row = dataclasses.asdict(grant)
    row["access"] = grant.access.value
    row["decision"] = grant.decision.value
    row["grant_id"] = grant.grant_id
    return row


def _emit(payload: dict[str, object]) -> None:
    print(json.dumps(payload, sort_keys=True, indent=2))


def _load_plan(path: Path) -> dict[str, object]:
    with path.open("r") as file:
        loaded = json.load(file)
    if not isinstance(loaded, dict):
        raise PlanError("plan file must contain a JSON object")
    return loaded


class _StdinConsent:
    """Interactive consent on the controlling terminal.

    Prints the exact access rule and the side-effect warning to stderr
    (stdout stays reserved for JSON results) and reads one decision.
    """

    _CHOICES = {
        "a": ConsentDecision.ALLOW_ONCE,
        "A": ConsentDecision.ALWAYS_ALLOW,
        "d": ConsentDecision.DENY_ONCE,
        "D": ConsentDecision.ALWAYS_DENY,
    }

    async def request_consent(
        self, request: AccessRequest, warning: str
    ) -> ConsentDecision:
        print(
            f"Consent required: {request.access.value} {request.resource} "
            f"offset {request.offset:#x} width {request.width} "
            f"(node {request.node_id}, digest {request.resource_digest})",
            file=sys.stderr,
        )
        print(warning, file=sys.stderr)
        while True:
            print(
                "[a] allow once  [A] always allow  [d] deny once  [D] always deny: ",
                end="",
                file=sys.stderr,
                flush=True,
            )
            choice = sys.stdin.readline().strip()
            decision = self._CHOICES.get(choice)
            if decision is not None:
                return decision
            print(f"unrecognized choice: {choice!r}", file=sys.stderr)


def _permissions_list(args: argparse.Namespace) -> int:
    grants = load_grants(args.grants)
    _emit({"grants": [_grant_row(grant) for grant in grants]})
    return EXIT_SUCCESS


def _permissions_revoke(args: argparse.Namespace) -> int:
    if not revoke_grant(args.grants, args.grant_id):
        print(
            json.dumps({"error": f"no grant with id {args.grant_id}"}),
            file=sys.stderr,
        )
        return EXIT_ERROR
    _emit({"revoked": args.grant_id})
    return EXIT_SUCCESS


def _permissions_explain(args: argparse.Namespace) -> int:
    request = AccessRequest(
        target_scope=args.target_scope,
        node_id=args.node_id,
        resource_digest=args.resource_digest,
        resource=args.resource,
        offset=args.offset,
        width=args.width,
        access=AccessClass(args.access),
    )
    resolution = resolve(request, load_grants(args.grants))
    _emit(
        {
            "outcome": resolution.outcome.value,
            "grant": _grant_row(resolution.grant) if resolution.grant else None,
        }
    )
    return EXIT_SUCCESS


def _permissions_add(args: argparse.Namespace) -> int:
    grant = ReadGrant(
        schema_version=1,
        target_scope=args.target_scope,
        node_id=args.node_id,
        resource_digest=args.resource_digest,
        resource=args.resource,
        offset=args.offset,
        width=args.width,
        access=AccessClass(args.access),
        decision=Decision(args.decision),
        approved_at=datetime.datetime.now(datetime.UTC).isoformat(),
        approval_source="cli",
    )
    add_grant(args.grants, grant)
    # The exact persisted rule, so the operator sees precisely what was
    # added -- never a region or wildcard.
    _emit({"added": _grant_row(grant)})
    return EXIT_SUCCESS


def _connect_transport(moniker: str, target: str | None) -> ProxyTransport:
    # Deferred import: the device connection path needs the
    # fuchsia-controller runtime, which permissions/plan commands and
    # their tests should not load. Tests substitute this seam.
    from driver_lab.fidl_transport import connect_transport

    return connect_transport(moniker=moniker, target=target)


def _run(args: argparse.Namespace) -> int:
    plan = _load_plan(args.plan)
    transport = _connect_transport(moniker=args.moniker, target=args.target)
    lab = DriverLab(
        transport,
        grants_path=args.grants,
        evidence_root=args.evidence_dir,
        target_scope=args.target_scope,
        node_id=args.node_id,
        consent=_StdinConsent() if args.consent else None,
    )
    result = asyncio.run(lab.run_plan(plan))
    _emit(
        {
            "exit_category": result.exit_category,
            "evidence_dir": str(result.evidence_dir),
            "plan_digest": result.plan_digest,
            "failure": result.failure,
            "reads": [dataclasses.asdict(read) for read in result.reads],
        }
    )
    return result.exit_category


def _plan_digest(args: argparse.Namespace) -> int:
    _emit({"digest": plan_digest(_load_plan(args.plan))})
    return EXIT_SUCCESS


def _plan_validate(args: argparse.Namespace) -> int:
    plan = _load_plan(args.plan)
    _emit({"digest": plan_digest(plan), "plan": validate_plan(plan)})
    return EXIT_SUCCESS


def _offset(value: str) -> int:
    return int(value, 0)


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="driver-lab")
    subcommands = parser.add_subparsers(dest="command", required=True)

    permissions = subcommands.add_parser(
        "permissions", help="manage persistent operator read grants"
    )
    permission_commands = permissions.add_subparsers(
        dest="subcommand", required=True
    )

    list_parser = permission_commands.add_parser("list", help="list grants")
    list_parser.add_argument("--grants", type=Path, required=True)
    list_parser.set_defaults(handler=_permissions_list)

    revoke_parser = permission_commands.add_parser(
        "revoke", help="revoke one grant"
    )
    revoke_parser.add_argument("--grants", type=Path, required=True)
    revoke_parser.add_argument("--grant-id", required=True)
    revoke_parser.set_defaults(handler=_permissions_revoke)

    explain_parser = permission_commands.add_parser(
        "explain", help="resolve one access request against the grants"
    )
    explain_parser.add_argument("--grants", type=Path, required=True)
    explain_parser.add_argument("--target-scope", required=True)
    explain_parser.add_argument("--node-id", required=True)
    explain_parser.add_argument("--resource-digest", required=True)
    explain_parser.add_argument("--resource", required=True)
    explain_parser.add_argument("--offset", type=_offset, required=True)
    explain_parser.add_argument("--width", type=int, default=4)
    explain_parser.add_argument(
        "--access",
        choices=[access.value for access in AccessClass],
        default=AccessClass.READ_ONCE.value,
    )
    explain_parser.set_defaults(handler=_permissions_explain)

    add_parser = permission_commands.add_parser(
        "add", help="persist one exact read grant or denial"
    )
    add_parser.add_argument("--grants", type=Path, required=True)
    add_parser.add_argument("--target-scope", required=True)
    add_parser.add_argument("--node-id", required=True)
    add_parser.add_argument("--resource-digest", required=True)
    add_parser.add_argument("--resource", required=True)
    add_parser.add_argument("--offset", type=_offset, required=True)
    add_parser.add_argument("--width", type=int, default=4)
    add_parser.add_argument(
        "--access",
        choices=[access.value for access in AccessClass],
        default=AccessClass.READ_ONCE.value,
    )
    add_parser.add_argument(
        "--decision",
        choices=[decision.value for decision in Decision],
        required=True,
    )
    add_parser.set_defaults(handler=_permissions_add)

    run_parser = subcommands.add_parser(
        "run", help="run one probe plan to a finalized evidence bundle"
    )
    run_parser.add_argument("--plan", type=Path, required=True)
    run_parser.add_argument("--evidence-dir", type=Path, required=True)
    run_parser.add_argument("--grants", type=Path, required=True)
    run_parser.add_argument("--target-scope", required=True)
    run_parser.add_argument("--node-id", required=True)
    run_parser.add_argument(
        "--moniker",
        required=True,
        help="component moniker of the proxy driver on the target",
    )
    run_parser.add_argument(
        "--target",
        default=None,
        help="target nodename (default: ffx default target)",
    )
    run_parser.add_argument(
        "--consent",
        action="store_true",
        help="prompt interactively for undecided accesses (default: fail closed)",
    )
    run_parser.set_defaults(handler=_run)

    plan = subcommands.add_parser(
        "plan", help="validate and digest probe plans"
    )
    plan_commands = plan.add_subparsers(dest="subcommand", required=True)

    digest_parser = plan_commands.add_parser(
        "digest", help="canonical plan digest"
    )
    digest_parser.add_argument("--plan", type=Path, required=True)
    digest_parser.set_defaults(handler=_plan_digest)

    validate_parser = plan_commands.add_parser(
        "validate", help="validate and canonicalize a plan"
    )
    validate_parser.add_argument("--plan", type=Path, required=True)
    validate_parser.set_defaults(handler=_plan_validate)

    return parser


def main(argv: Sequence[str] | None = None) -> int:
    """Runs one CLI command; returns the exit category."""
    args = _build_parser().parse_args(argv)
    try:
        handler = args.handler
        result: int = handler(args)
        return result
    except EvidenceError as error:
        print(json.dumps({"error": str(error)}), file=sys.stderr)
        return EXIT_EVIDENCE
    except KeyboardInterrupt:
        # Cancellation inside a run finalizes evidence and reports
        # through the run result; reaching here means the interrupt
        # arrived outside that path.
        print(json.dumps({"error": "interrupted"}), file=sys.stderr)
        return EXIT_OPERATION
    except (
        GrantStoreError,
        PlanError,
        OSError,
        ValueError,
        json.JSONDecodeError,
    ) as error:
        print(json.dumps({"error": str(error)}), file=sys.stderr)
        return EXIT_ERROR


if __name__ == "__main__":
    sys.exit(main())
