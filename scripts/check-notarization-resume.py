"""Authorize recovery artifacts against the repository's exact release tag."""
import json
import os
import re
import subprocess
import sys


def api(route):
    environment = dict(os.environ)
    for name in ("FORCE_COLOR", "CLICOLOR_FORCE", "GH_FORCE_TTY"):
        environment.pop(name, None)
    environment["NO_COLOR"] = "1"
    result = subprocess.run(["gh", "api", route], check=True, capture_output=True, text=True, env=environment)
    return json.loads(result.stdout)


def main():
    run_id, tag = sys.argv[1:3]
    repository = os.environ["GITHUB_REPOSITORY"]
    if not re.fullmatch(r"[0-9]{1,20}", run_id) or not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag):
        raise ValueError("Expected a numeric run ID and a stable version tag such as v0.6.3")
    run = api(f"repos/{repository}/actions/runs/{run_id}")
    if (run["head_repository"]["full_name"] != repository or run["event"] != "push"
            or run["head_branch"] != tag or run["path"] != ".github/workflows/release.yml"
            or run["status"] != "completed"):
        raise ValueError("Source must be a completed Build and Release tag-push run in this repository")
    reference = api(f"repos/{repository}/git/ref/tags/{tag}")["object"]
    for _ in range(5):
        if reference["type"] != "tag":
            break
        reference = api(f"repos/{repository}/git/tags/{reference['sha']}")["object"]
    if reference["type"] != "commit" or reference["sha"] != run["head_sha"]:
        raise ValueError("Release tag no longer identifies the original build commit")
    sha = reference["sha"]
    if not re.fullmatch(r"[a-f0-9]{40}", sha):
        raise ValueError("Invalid source commit")
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        output.write(f"source_sha={sha}\nsource_run_id={run_id}\nrelease_tag={tag}\n")
    print(f"Verified recovery source: {repository} {tag} ({sha}), run {run_id}")


if __name__ == "__main__":
    try:
        main()
    except (KeyError, ValueError, OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"Cannot resume notarization: {error}")
