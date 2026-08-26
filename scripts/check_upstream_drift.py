#!/usr/bin/env python3
"""Detect drift between rmkit and upstream RMK.

Two axes, checked independently:

  release   rmkit's latest crates.io release  vs  rmk's latest crates.io release
  main      rmkit's working tree              vs  rmk's `main` branch

They answer different questions and drift for different reasons. The release
axis asks whether what users can install today works with the RMK they can
install today. The main axis asks whether the *next* rmkit release will work
with the *next* RMK release. A green release axis and a red main axis is the
normal state while RMK has unreleased changes.

Stdlib only (urllib + tomllib), so it runs on a bare CI image and locally with
no setup. Python >= 3.11.

Usage:
    python3 scripts/check_upstream_drift.py                 # both axes
    python3 scripts/check_upstream_drift.py --axis main
    python3 scripts/check_upstream_drift.py --format json
"""

from __future__ import annotations

import argparse
import io
import json
import os
import re
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

RMK_REPO = "rmk-rs/rmk"
RMKIT_REPO = "rmk-rs/rmkit"
TEMPLATE_REPO = "rmk-rs/rmk-template"

USER_AGENT = "rmkit-upstream-drift-check"

# Crates rmkit depends on that live in the rmk workspace, mapped to their
# manifest path inside an rmk tree. Each one pins rmkit to a point in rmk's
# history, so each one can drift. Add a crate here when rmkit starts depending
# on it.
TRACKED_DEPS = {
    "rmk-config": "rmk-config/Cargo.toml",
    "rynk-kle": "rynk/rynk-kle/Cargo.toml",
}

# Example keyboard.toml files that rmkit is not expected to parse. Add entries
# here (with a reason) rather than weakening the check.
EXPECTED_PARSE_FAILURES: dict[str, str] = {}


# --------------------------------------------------------------------------
# Findings
# --------------------------------------------------------------------------


@dataclass
class Finding:
    level: str  # "fail" | "warn" | "info"
    check: str
    message: str


@dataclass
class Report:
    axis: str
    # Whether this axis can fail the run. The release axis describes artifacts
    # that are already published: nothing you do in the working tree changes
    # its verdict, so it reports but never gates.
    gating: bool = True
    findings: list[Finding] = field(default_factory=list)

    def fail(self, check: str, message: str) -> None:
        self.findings.append(Finding("fail", check, message))

    def warn(self, check: str, message: str) -> None:
        self.findings.append(Finding("warn", check, message))

    def info(self, check: str, message: str) -> None:
        self.findings.append(Finding("info", check, message))

    @property
    def failed(self) -> bool:
        return any(f.level == "fail" for f in self.findings)


# --------------------------------------------------------------------------
# Fetching
# --------------------------------------------------------------------------


def http_get(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    token = os.environ.get("GITHUB_TOKEN")
    if token and "github.com" in url:
        request.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(request, timeout=120) as response:
        return response.read()


def http_get_json(url: str):
    return json.loads(http_get(url))


def crates_io_newest(crate: str) -> str:
    data = http_get_json(f"https://crates.io/api/v1/crates/{crate}")
    return data["crate"]["newest_version"]


def crates_io_published_versions(crate: str) -> list[str]:
    data = http_get_json(f"https://crates.io/api/v1/crates/{crate}")
    return [v["num"] for v in data["versions"] if not v.get("yanked")]


def fetch_rmk_tree(ref: str, kind: str, dest: Path) -> Path:
    """Download an rmk tree (branch or tag) and return its extracted root."""
    url = f"https://codeload.github.com/{RMK_REPO}/tar.gz/refs/{kind}/{ref}"
    with tarfile.open(fileobj=io.BytesIO(http_get(url)), mode="r:gz") as archive:
        archive.extractall(dest)
    roots = [p for p in dest.iterdir() if p.is_dir()]
    if len(roots) != 1:
        raise RuntimeError(f"unexpected archive layout for {ref}: {roots}")
    return roots[0]


def resolve_rmk_tag(version: str) -> str:
    """RMK tagged releases as `v0.6.0` early on and `rmk-v0.8.2` since."""
    for candidate in (f"rmk-v{version}", f"v{version}"):
        try:
            http_get(f"https://api.github.com/repos/{RMK_REPO}/git/ref/tags/{candidate}")
            return candidate
        except urllib.error.HTTPError as err:
            if err.code != 404:
                raise
    raise RuntimeError(f"no git tag found for rmk {version}")


# --------------------------------------------------------------------------
# Reading rmkit's own assumptions out of its source
# --------------------------------------------------------------------------
#
# These parse rmkit's Rust source rather than duplicating its constants here,
# so the checker cannot silently fall out of sync with the code it checks. If
# an anchor stops matching the extractor raises — that is the intended
# behaviour, a refactor should force a look at this file.


def extract_dep_reqs(cargo_toml: str) -> dict[str, str]:
    """Read rmkit's requirement on each tracked rmk-workspace crate."""
    dependencies = tomllib.loads(cargo_toml)["dependencies"]
    reqs: dict[str, str] = {}
    for name in TRACKED_DEPS:
        dep = dependencies.get(name)
        if dep is None:
            continue  # rmkit did not depend on this crate at this revision
        if isinstance(dep, str):
            reqs[name] = dep
        elif "version" in dep:
            reqs[name] = dep["version"]
        elif "git" in dep:
            reqs[name] = f"git:{dep.get('rev', dep.get('branch', 'HEAD'))}"
        else:
            raise RuntimeError(f"cannot read a version requirement for {name} from {dep!r}")
    if not reqs:
        raise RuntimeError(f"rmkit depends on none of {sorted(TRACKED_DEPS)}")
    return reqs


def extract_feature_names(keyboard_toml_rs: str) -> tuple[set[str], set[str]]:
    """Return (features rmkit disables, features rmkit enables)."""
    disabled = set(re.findall(r'disabled_default_feature\.push\("([^"]+)"', keyboard_toml_rs))
    enabled = set(re.findall(r'enabled_feature\.push\("([^"]+)"', keyboard_toml_rs))
    if not disabled:
        raise RuntimeError("no `disabled_default_feature.push(..)` calls found in keyboard_toml.rs")
    return disabled, enabled


def extract_chip_options(chip_rs: str) -> tuple[list[str], list[str]]:
    """Return (split chip options, unibody chip options)."""
    match = re.search(
        r"fn get_chip_options.*?if split \{\s*vec!\[(.*?)\]\s*\} else \{\s*vec!\[(.*?)\]",
        chip_rs,
        re.DOTALL,
    )
    if not match:
        raise RuntimeError("could not locate the two vec![] blocks in get_chip_options")
    return (
        re.findall(r'"([^"]+)"', match.group(1)),
        re.findall(r'"([^"]+)"', match.group(2)),
    )


def extract_board_chip_map(chip_rs: str) -> dict[str, str]:
    mapping = dict(re.findall(r'map\.insert\("([^"]+)",\s*"([^"]+)"\)', chip_rs))
    if not mapping:
        raise RuntimeError("no `map.insert(..)` calls found in get_board_chip_map")
    return mapping


# --------------------------------------------------------------------------
# Cargo version requirements
# --------------------------------------------------------------------------


def parse_version(version: str) -> tuple[int, int, int]:
    core = version.split("-")[0].split("+")[0]
    parts = [int(p) for p in core.split(".")]
    while len(parts) < 3:
        parts.append(0)
    return parts[0], parts[1], parts[2]


def req_admits(req: str, version: str) -> bool:
    """Whether a Cargo version requirement admits a concrete version.

    Only the forms RMK and rmkit actually use: `=X.Y.Z` and bare/caret.
    """
    req = req.strip()
    target = parse_version(version)
    if req.startswith("="):
        return parse_version(req[1:]) == target
    bare = req.lstrip("^")
    lower = parse_version(bare)
    if target < lower:
        return False
    # Caret: the leftmost non-zero component may not change.
    major, minor, _ = lower
    if major > 0:
        return target[0] == major
    if minor > 0:
        return target[0] == 0 and target[1] == minor
    return target[0] == 0 and target[1] == 0


# --------------------------------------------------------------------------
# Upstream facts, per axis
# --------------------------------------------------------------------------


@dataclass
class Upstream:
    """What the rmk side of an axis looks like."""

    label: str
    rmk_version: str
    rmk_types_version: str
    dep_versions: dict[str, str]
    features: dict[str, list[str]]
    example_configs: list[Path]


def crate_version(tree: Path, manifest_path: str) -> str | None:
    """None when the crate does not exist in this tree — older rmk releases
    predate some of the workspace members rmkit now depends on."""
    manifest = tree / manifest_path
    if not manifest.is_file():
        return None
    version = tomllib.loads(manifest.read_text())["package"]["version"]
    if isinstance(version, str):
        return version
    # `version.workspace = true` — resolve against the nearest ancestor
    # manifest that defines `[workspace.package]`. rynk's members do this.
    for directory in manifest.parent.parents:
        candidate = directory / "Cargo.toml"
        if not candidate.is_file():
            continue
        inherited = tomllib.loads(candidate.read_text()).get("workspace", {}).get("package", {})
        if "version" in inherited:
            return inherited["version"]
        if directory == tree:
            break
    raise RuntimeError(f"cannot resolve the workspace version for {manifest_path}")


def read_upstream(tree: Path, label: str) -> Upstream:
    rmk_manifest = tomllib.loads((tree / "rmk" / "Cargo.toml").read_text())
    return Upstream(
        label=label,
        rmk_version=rmk_manifest["package"]["version"],
        rmk_types_version=crate_version(tree, "rmk-types/Cargo.toml"),
        dep_versions={
            name: version
            for name, path in TRACKED_DEPS.items()
            if (version := crate_version(tree, path)) is not None
        },
        features=rmk_manifest.get("features", {}),
        # `use_config` only: those are the full keyboard.toml files rmkit is
        # built to consume. The `use_rust` examples carry partial configs with
        # no `[keyboard]` section — rmk's build.rs reads them through
        # `new_from_toml_path_with_event_defaults`, and rmkit never sees them.
        example_configs=sorted((tree / "examples" / "use_config").rglob("keyboard.toml")),
    )


# --------------------------------------------------------------------------
# Checks
# --------------------------------------------------------------------------


DRIFT_CONSEQUENCE = {
    "rmk-config": "rmkit parses a different keyboard.toml schema than the projects it generates",
    "rynk-kle": "`rmkit layout` converts against a different layout format than rmk understands",
}


def check_versions(report: Report, reqs: dict[str, str], upstream: Upstream) -> None:
    shipped = " · ".join(f"{name} {version}" for name, version in upstream.dep_versions.items())
    report.info(
        "versions",
        f"rmk {upstream.rmk_version} · rmk-types {upstream.rmk_types_version} "
        f"· {shipped} ({upstream.label})",
    )
    report.info(
        "versions",
        "rmkit requires " + ", ".join(f"{name} {req}" for name, req in reqs.items()),
    )

    for name, req in reqs.items():
        if req.startswith("git:"):
            report.warn(
                "versions",
                f"rmkit depends on {name} via git ({req[4:]}); it cannot be published "
                "in this state",
            )
            continue
        shipped_version = upstream.dep_versions.get(name)
        if shipped_version is None:
            report.fail(
                "versions",
                f"rmkit requires {name}, but {upstream.label} has no such crate",
            )
            continue
        if not req_admits(req, shipped_version):
            report.fail(
                "versions",
                f"rmkit requires {name} {req}, but {upstream.label} ships {name} "
                f"{shipped_version} — {DRIFT_CONSEQUENCE.get(name, 'rmkit is out of step')}",
            )


def check_features(report: Report, disabled: set[str], enabled: set[str], upstream: Upstream) -> None:
    available = set(upstream.features)
    defaults = set(upstream.features.get("default", []))

    for name in sorted(disabled | enabled):
        if name not in available:
            report.fail(
                "features",
                f"rmkit emits the cargo feature `{name}`, which does not exist in "
                f"{upstream.label} (rmk {upstream.rmk_version})",
            )

    # rmkit only removes names from the default set; one that is not a default
    # is a silent no-op rather than an error, so it is a warning.
    for name in sorted(disabled):
        if name in available and name not in defaults:
            report.warn(
                "features",
                f"rmkit tries to disable `{name}`, but it is not a default feature in "
                f"{upstream.label} — that disable is a no-op",
            )

    if not report.failed:
        report.info("features", f"{len(disabled | enabled)} feature names verified against {upstream.label}")


def summarise_failure(output: str) -> str:
    """Pull the useful line out of a Rust panic, not the `thread 'main'` banner."""
    for line in output.strip().splitlines():
        line = line.strip()
        if not line or line.startswith(("thread '", "note:", "stack backtrace")):
            continue
        return line
    return "no output"


def check_config_schema(report: Report, rmkit_bin: Path, upstream: Upstream) -> None:
    """Parse every upstream example keyboard.toml with the real rmkit binary."""
    failures: list[str] = []
    checked = 0
    for config in upstream.example_configs:
        name = str(config).split("/examples/", 1)[-1]
        if name in EXPECTED_PARSE_FAILURES:
            continue
        checked += 1
        result = subprocess.run(
            [str(rmkit_bin), "get-chip", "--keyboard-toml-path", str(config)],
            capture_output=True,
            text=True,
            cwd=tempfile.gettempdir(),
        )
        if result.returncode != 0 or not result.stdout.strip():
            failures.append(f"{name}: {summarise_failure(result.stderr or result.stdout)}")

    if failures:
        report.fail(
            "config-schema",
            f"rmkit cannot parse {len(failures)}/{checked} example keyboard.toml files "
            f"from {upstream.label}:\n    " + "\n    ".join(failures[:10]),
        )
    else:
        report.info("config-schema", f"parsed {checked} example keyboard.toml files from {upstream.label}")


def check_templates(report: Report, split_chips: list[str], unibody_chips: list[str], board_map: dict[str, str]) -> None:
    listing = http_get_json(f"https://api.github.com/repos/{TEMPLATE_REPO}/contents/")
    folders = {entry["name"] for entry in listing if entry["type"] == "dir" and not entry["name"].startswith(".")}

    def resolves(chip: str, split: bool) -> bool:
        chip = board_map.get(chip, chip)
        folder = f"{chip}_split" if split else chip
        if folder in folders:
            return True
        # rmkit falls back to the stm32 family folder, then to plain `stm32`.
        if folder.startswith("stm32"):
            return folder[:7] in folders or "stm32" in folders
        return False

    for chip in unibody_chips:
        if not resolves(chip, split=False):
            report.fail("templates", f"`rmkit init --chip {chip}` has no template in {TEMPLATE_REPO}")
    for chip in split_chips:
        if not resolves(chip, split=True):
            report.fail("templates", f"`rmkit init --chip {chip} --split true` has no template in {TEMPLATE_REPO}")

    offered = {board_map.get(c, c) for c in unibody_chips} | {f"{board_map.get(c, c)}_split" for c in split_chips}
    for folder in sorted(folders - offered):
        if folder.startswith("stm32"):
            continue  # covered by the stm32 fallback, not enumerated
        report.warn("templates", f"{TEMPLATE_REPO} has a `{folder}` template that `rmkit init` never offers")

    if not any(f.check == "templates" and f.level == "fail" for f in report.findings):
        report.info("templates", f"{len(split_chips) + len(unibody_chips)} chip options resolve to a template")


def check_version_mapping(report: Report) -> None:
    mapping = http_get_json(
        f"https://raw.githubusercontent.com/{TEMPLATE_REPO}/main/version-mapping.json"
    )
    published = {".".join(v.split(".")[:2]) for v in crates_io_published_versions("rmk")}
    # Only minors newer than the newest mapped one matter. Everything older
    # predates the mapping scheme and is never getting an entry.
    floor = max((parse_version(v) for v in mapping), default=(0, 0, 0))
    missing = sorted(
        (v for v in published - set(mapping) if parse_version(v) > floor),
        key=parse_version,
    )
    if missing:
        report.warn(
            "version-mapping",
            f"{TEMPLATE_REPO}/version-mapping.json has no entry for published rmk "
            f"{', '.join(missing)} — `rmkit create --version {missing[-1]}` will be rejected",
        )
    else:
        report.info(
            "version-mapping",
            f"newest mapped rmk minor is {'.'.join(str(p) for p in floor[:2])}, "
            "which is current with crates.io",
        )


# --------------------------------------------------------------------------
# Axes
# --------------------------------------------------------------------------


def build_rmkit() -> Path:
    subprocess.run(["cargo", "build", "--quiet"], cwd=REPO_ROOT, check=True)
    return REPO_ROOT / "target" / "debug" / "rmkit"


def run_main_axis(workdir: Path) -> Report:
    report = Report("main")
    tree = fetch_rmk_tree("main", "heads", workdir / "rmk-main")
    upstream = read_upstream(tree, "rmk main")

    cargo_toml = (REPO_ROOT / "Cargo.toml").read_text()
    keyboard_toml_rs = (REPO_ROOT / "src" / "keyboard_toml.rs").read_text()
    chip_rs = (REPO_ROOT / "src" / "chip.rs").read_text()

    disabled, enabled = extract_feature_names(keyboard_toml_rs)
    split_chips, unibody_chips = extract_chip_options(chip_rs)

    check_versions(report, extract_dep_reqs(cargo_toml), upstream)
    check_features(report, disabled, enabled, upstream)
    check_config_schema(report, build_rmkit(), upstream)
    check_templates(report, split_chips, unibody_chips, extract_board_chip_map(chip_rs))
    return report


def run_release_axis(workdir: Path) -> Report:
    report = Report("release", gating=False)

    rmkit_version = crates_io_newest("rmkit")
    rmk_version = crates_io_newest("rmk")
    report.info("versions", f"rmkit {rmkit_version} (crates.io) vs rmk {rmk_version} (crates.io)")

    tag = resolve_rmk_tag(rmk_version)
    tree = fetch_rmk_tree(tag, "tags", workdir / "rmk-release")
    upstream = read_upstream(tree, f"rmk {rmk_version}")

    def rmkit_file(path: str) -> str:
        return http_get(
            f"https://raw.githubusercontent.com/{RMKIT_REPO}/v{rmkit_version}/{path}"
        ).decode()

    cargo_toml = rmkit_file("Cargo.toml")
    keyboard_toml_rs = rmkit_file("src/keyboard_toml.rs")
    chip_rs = rmkit_file("src/chip.rs")

    disabled, enabled = extract_feature_names(keyboard_toml_rs)
    split_chips, unibody_chips = extract_chip_options(chip_rs)

    check_versions(report, extract_dep_reqs(cargo_toml), upstream)
    check_features(report, disabled, enabled, upstream)
    # No config-schema check here: it is subsumed by the version check. If the
    # released rmkit and the released rmk resolve the same rmk-config, they
    # cannot disagree about the schema. Only the main axis can have matching
    # version numbers with differing content.
    check_templates(report, split_chips, unibody_chips, extract_board_chip_map(chip_rs))
    return report


# --------------------------------------------------------------------------
# Output
# --------------------------------------------------------------------------

ICONS = {"fail": "✗", "warn": "!", "info": "·"}


def reconcile(release: Report, main: Report) -> None:
    """Annotate release-axis failures with whether main has already fixed them.

    A finding on both axes is a live bug. One that only shows on the release
    axis is already fixed in the working tree and just needs a release — very
    different calls to action, so say which it is.
    """
    live = {(f.check, f.message) for f in main.findings if f.level == "fail"}
    for finding in release.findings:
        if finding.level != "fail":
            continue
        if (finding.check, finding.message) in live:
            finding.message += "\n→ still broken on main; fix it before releasing"
        else:
            finding.message += "\n→ already fixed on main; ships with the next release"


def print_report(report: Report) -> None:
    scope = "" if report.gating else "  (reports only, never gates)"
    print(f"\n=== {report.axis} axis ==={scope}")
    for finding in report.findings:
        icon = ICONS[finding.level] if report.gating else ICONS.get(finding.level, "!").replace("✗", "!")
        head, *rest = finding.message.split("\n")
        print(f"  {icon} [{finding.check}] {head}")
        for line in rest:
            print(f"      {line}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--axis", choices=["main", "release", "both"], default="both")
    parser.add_argument("--format", choices=["text", "json"], default="text")
    args = parser.parse_args()

    reports: list[Report] = []
    with tempfile.TemporaryDirectory() as tmp:
        workdir = Path(tmp)
        release = run_release_axis(workdir) if args.axis in ("release", "both") else None
        main_ = run_main_axis(workdir) if args.axis in ("main", "both") else None
        if release and main_:
            reconcile(release, main_)
        reports = [r for r in (release, main_) if r]

        shared = Report("shared")
        check_version_mapping(shared)
        reports.append(shared)

    gating_failures = [r.axis for r in reports if r.gating and r.failed]

    if args.format == "json":
        print(json.dumps(
            {
                "drifted": bool(gating_failures),
                "reports": [
                    {"axis": r.axis, "gating": r.gating, "findings": [vars(f) for f in r.findings]}
                    for r in reports
                ],
            },
            indent=2,
        ))
    else:
        for report in reports:
            print_report(report)
        print()
        if gating_failures:
            print(f"DRIFT DETECTED on: {', '.join(gating_failures)}")
        elif any(r.failed for r in reports):
            print("No drift on gating axes; see the release axis for what a release would fix.")
        else:
            print("No drift detected.")

    return 1 if gating_failures else 0


if __name__ == "__main__":
    sys.exit(main())
