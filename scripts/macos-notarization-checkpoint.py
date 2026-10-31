"""Non-secret, immutable checkpoint for an already submitted signed release."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import posixpath
import re
import stat
import sys
import uuid
import zipfile


def digest(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError("Checkpoint files must be regular files")
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def context():
    return {
        "repository": os.environ["GITHUB_REPOSITORY"],
        "sourceSha": os.environ.get("EXPECTED_SOURCE_SHA") or os.environ["GITHUB_SHA"],
        "ref": os.environ.get("EXPECTED_RELEASE_TAG") or os.environ["GITHUB_REF_NAME"],
    }


def inspect_archive(path, app_name):
    if PurePosixPath(app_name).name != app_name or not app_name.endswith(".app"):
        raise ValueError("Invalid application name")
    with zipfile.ZipFile(path) as archive:
        if not archive.infolist():
            raise ValueError("Empty application archive")
        for entry in archive.infolist():
            name = PurePosixPath(entry.filename)
            if name.is_absolute() or ".." in name.parts or name.parts[0] not in (app_name, "__MACOSX"):
                raise ValueError("Unsafe application archive path")
            if stat.S_ISLNK(entry.external_attr >> 16):
                target = archive.read(entry).decode("utf-8")
                resolved = posixpath.normpath(posixpath.join(str(name.parent), target))
                if not (resolved == app_name or resolved.startswith(app_name + "/")):
                    raise ValueError("Application symlink escapes archive")


def main():
    action, directory, target, arch = sys.argv[1:5]
    if (target, arch) not in (("x86_64-apple-darwin", "x64"), ("aarch64-apple-darwin", "arm64")):
        raise ValueError("Unsupported checkpoint architecture")
    root = Path(directory)
    dmg = root / f"Luna-Mux_macos-{arch}.dmg"
    app_archive = root / "app.zip"
    manifest = root / "manifest.json"
    if action == "create":
        receipt_path, app_name = sys.argv[5:7]
        receipt = json.loads(Path(receipt_path).read_text())
        submission_id = str(uuid.UUID(receipt["id"]))
        inspect_archive(app_archive, app_name)
        value = {
            "schemaVersion": 1, **context(), "target": target, "arch": arch,
            "appName": app_name, "submissionId": submission_id,
            "dmgSha256": digest(dmg), "appSha256": digest(app_archive),
        }
        manifest.write_text(json.dumps(value, indent=2) + "\n")
    elif action == "validate":
        if manifest.is_symlink():
            raise ValueError("Invalid checkpoint manifest")
        value = json.loads(manifest.read_text())
        expected_keys = {
            "schemaVersion", "repository", "sourceSha", "ref", "target", "arch",
            "appName", "submissionId", "dmgSha256", "appSha256",
        }
        if set(value) != expected_keys or value["schemaVersion"] != 1:
            raise ValueError("Unsupported checkpoint manifest")
        for key, expected in {**context(), "target": target, "arch": arch}.items():
            if value[key] != expected:
                raise ValueError(f"Checkpoint {key} does not match requested release")
        if not re.fullmatch(r"[a-f0-9]{40}", value["sourceSha"]):
            raise ValueError("Invalid checkpoint commit")
        if digest(dmg) != value["dmgSha256"] or digest(app_archive) != value["appSha256"]:
            raise ValueError("Checkpoint content was modified")
        inspect_archive(app_archive, value["appName"])
        submission_id = str(uuid.UUID(value["submissionId"]))
    else:
        raise ValueError("Unknown checkpoint command")
    print(submission_id)


if __name__ == "__main__":
    try:
        main()
    except (KeyError, ValueError, OSError, zipfile.BadZipFile) as error:
        raise SystemExit(f"Invalid notarization checkpoint: {error}")
