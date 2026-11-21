#!/usr/bin/env python3
"""Where the 750-image corpus comes from.

Every image in the corpus is fetched from a public repository on GitHub. A
source records the repository, the ref it was read at, the paths inside it,
the category the images belong to, the licence, and who to attribute. The
builder resolves the ref to a commit, so a manifest entry always names the
exact bytes it came from.

Three rules keep the corpus honest:

* **Nothing is redistributed.** The image bytes live under the git-ignored
  ``corpus-data/`` tree. Only the digests, the dimensions and the provenance
  are committed.
* **The category is a fact about the source, not a guess.** An image from a
  sprite repository is pixel art only if the pixel-art test passes; the test
  is recorded with the image.
* **The selection is deterministic.** Candidates are ordered by the SHA-256 of
  their path, so two runs over the same commit take the same images in the
  same order.

The repositories are used as *source material for a local benchmark*. Their
own licences govern the images; nothing here re-licenses them.
"""

from __future__ import annotations

import hashlib
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass

API_ROOT = "https://api.github.com"
RAW_ROOT = "https://raw.githubusercontent.com"
USER_AGENT = (
    "VPNG-corpus/0.3 (local benchmark corpus; provenance in "
    "benchmarks/corpus_manifest.json)"
)

#: The image file extensions the builder will consider. Pillow decides
#: whether it can actually decode the ones that survive this filter.
IMAGE_EXTENSIONS = frozenset(
    {
        ".bmp",
        ".gif",
        ".j2k",
        ".jp2",
        ".jpe",
        ".jpeg",
        ".jpg",
        ".png",
        ".ppm",
        ".tga",
        ".tif",
        ".tiff",
        ".webp",
    }
)

#: The five categories the corpus is built around.
CATEGORIES: dict[str, str] = {
    "photos": "Natural photographs: animals, objects, vehicles, people.",
    "ui": "Application screenshots: editor chrome, menus, text, icons.",
    "pixel_art": (
        "Pixel art and low-resolution sprite art: few colours, hard edges, "
        "small canvases."
    ),
    "scans": (
        "Digitised and instrument captures: photographed print, scanned "
        "pages, camera and scientific captures."
    ),
    "textures": (
        "Tiling textures and surface detail: brick, wood, fabric, ground, "
        "gradients, noise."
    ),
}

#: How many images each category holds in the 750-image corpus.
CATEGORY_TARGETS: dict[str, int] = {
    "photos": 300,
    "ui": 100,
    "pixel_art": 100,
    "scans": 100,
    "textures": 150,
}

#: The categories the first manifest used, and where they belong now. The
#: seed images are kept; they are not thrown away to fit a new taxonomy.
CATEGORY_MIGRATION = {
    "photographs": "photos",
    "screenshots": "ui",
    "anime": "pixel_art",
    "digital_paintings": "scans",
    "renders": "textures",
    "procedural": "textures",
    "noise": "textures",
}


def migrate_manifest(manifest: dict) -> dict:
    """Move a manifest onto the five-category taxonomy, in place.

    The first manifest had seven categories. The categories are merged, not
    renamed one for one, so the merge is recorded rather than implied: two
    old categories become one new one.
    """
    for category in manifest.get("categories", []):
        old = category.get("id")
        if old in CATEGORY_MIGRATION:
            category["id"] = CATEGORY_MIGRATION[old]
    for entry in manifest.get("images", []):
        old = entry.get("category")
        if old in CATEGORY_MIGRATION:
            entry["category"] = CATEGORY_MIGRATION[old]
    return manifest


@dataclass(frozen=True)
class Source:
    """One repository the corpus draws images from."""

    id: str
    repo: str
    ref: str
    paths: tuple[str, ...]
    category: str
    license: str
    attribution: str
    notes: str
    limit: int
    #: ``"any"`` takes every decodable image; ``"pixel_art"`` takes only the
    #: images that pass the pixel-art test.
    accept: str = "any"

    @property
    def url(self) -> str:
        return f"https://github.com/{self.repo}"

    def covers(self, path: str) -> bool:
        """True when `path` is inside one of the source's directories."""
        for prefix in self.paths:
            if prefix == "":
                return True
            if path == prefix or path.startswith(prefix + "/"):
                return True
        return False

    def to_dict(self, commit: str) -> dict:
        return {
            "id": self.id,
            "repo": self.repo,
            "ref": self.ref,
            "commit": commit,
            "paths": list(self.paths),
            "category": self.category,
            "license": self.license,
            "attribution": self.attribution,
            "notes": self.notes,
            "limit": self.limit,
            "accept": self.accept,
            "url": self.url,
        }


#: The source registry. The order matters: the builder fills each category
#: from the sources in this order until the category's target is met.
SOURCES: tuple[Source, ...] = (
    Source(
        id="imagenet-sample-images",
        repo="EliSchwartz/imagenet-sample-images",
        ref="master",
        paths=("",),
        category="photos",
        license="Varies by image; see the upstream README and the ImageNet terms",
        attribution="ImageNet sample images, collected in EliSchwartz/imagenet-sample-images",
        notes="One thousand JPEG photographs of ImageNet classes.",
        limit=400,
    ),
    Source(
        id="opencv-samples-data",
        repo="opencv/opencv",
        ref="4.x",
        paths=("samples/data",),
        category="photos",
        license="Apache-2.0 for the repository; the sample images carry their own terms",
        attribution="OpenCV sample data",
        notes="Classic sample photographs; a few synthetic images are tagged as such.",
        limit=120,
    ),
    Source(
        id="github-docs-images",
        repo="github/docs",
        ref="main",
        paths=("assets/images",),
        category="ui",
        license="Mixed; see the upstream LICENSE.md (documentation is CC BY 4.0)",
        attribution="GitHub Docs images",
        notes="Product screenshots from the GitHub documentation.",
        limit=200,
    ),
    Source(
        id="vscode-docs-images",
        repo="microsoft/vscode-docs",
        ref="main",
        paths=("release-notes/images",),
        category="ui",
        license="Mixed; Microsoft documentation assets",
        attribution="Visual Studio Code documentation images",
        notes="Editor screenshots from the VS Code release notes.",
        limit=200,
    ),
    Source(
        id="phaser-example-sprites",
        repo="photonstorm/phaser-examples",
        ref="master",
        paths=("examples/assets/sprites",),
        category="pixel_art",
        license="Mixed; Phaser example assets",
        attribution="Phaser example sprites",
        notes="Sprite sheets from the Phaser examples.",
        limit=200,
        accept="pixel_art",
    ),
    Source(
        id="phaser-example-bullets",
        repo="photonstorm/phaser-examples",
        ref="master",
        paths=("examples/assets/bullets",),
        category="pixel_art",
        license="Mixed; Phaser example assets",
        attribution="Phaser example bullet sprites",
        notes="Small projectile sprites from the Phaser examples.",
        limit=200,
        accept="pixel_art",
    ),
    Source(
        id="phaser-example-particles",
        repo="photonstorm/phaser-examples",
        ref="master",
        paths=("examples/assets/particlestorm",),
        category="pixel_art",
        license="Mixed; Phaser example assets",
        attribution="Phaser example particle sprites",
        notes="Particle sprites from the Phaser examples.",
        limit=200,
        accept="pixel_art",
    ),
    Source(
        id="opencv-pngsuite",
        repo="opencv/opencv_extra",
        ref="4.x",
        paths=("testdata/highgui/pngsuite",),
        category="pixel_art",
        license="Mixed; OpenCV test data",
        attribution="OpenCV PNG test suite",
        notes="The classic PNG test suite: small computer-generated images.",
        limit=200,
        accept="pixel_art",
    ),
    Source(
        id="opencv-calibration-photos",
        repo="opencv/opencv_extra",
        ref="4.x",
        paths=("testdata/cv/cameracalibration",),
        category="scans",
        license="Mixed; OpenCV test data",
        attribution="OpenCV camera-calibration photographs",
        notes="Photographs of printed calibration patterns.",
        limit=120,
    ),
    Source(
        id="opencv-readwrite-images",
        repo="opencv/opencv_extra",
        ref="4.x",
        paths=("testdata/highgui/readwrite",),
        category="scans",
        license="Mixed; OpenCV test data",
        attribution="OpenCV read/write test images",
        notes="Scanned and photographed documents used by the codec tests.",
        limit=120,
    ),
    Source(
        id="scikit-image-data",
        repo="scikit-image/scikit-image",
        ref="main",
        paths=("src/_skimage2",),
        category="scans",
        license="BSD-3-Clause for the repository; the data files carry their own terms",
        attribution="scikit-image sample data",
        notes="Digitised pages, instrument captures and scientific images.",
        limit=120,
    ),
    Source(
        id="threejs-textures",
        repo="mrdoob/three.js",
        ref="dev",
        paths=("examples/textures",),
        category="textures",
        license="MIT for the repository; the example assets carry their own terms",
        attribution="three.js example textures",
        notes="Tiling textures, normal maps and surface photographs.",
        limit=400,
    ),
    Source(
        id="godot-decal-textures",
        repo="godotengine/godot-demo-projects",
        ref="master",
        paths=("3d/decals/textures",),
        category="textures",
        license="MIT for the repository; the demo assets carry their own terms",
        attribution="Godot demo project decal textures",
        notes="Decal and surface textures from the Godot demos.",
        limit=100,
    ),
    Source(
        id="godot-material-textures",
        repo="godotengine/godot-demo-projects",
        ref="master",
        paths=("3d/material_testers/test_materials",),
        category="textures",
        license="MIT for the repository; the demo assets carry their own terms",
        attribution="Godot demo project material textures",
        notes="Material test textures from the Godot demos.",
        limit=100,
    ),
    Source(
        id="phaser-example-pictures",
        repo="photonstorm/phaser-examples",
        ref="master",
        paths=("examples/assets/pics",),
        category="pixel_art",
        license="Mixed; Phaser example assets",
        attribution="Phaser example pictures",
        notes="Small pictures used by the Phaser examples.",
        limit=200,
        accept="pixel_art",
    ),
    Source(
        id="phaser-example-games",
        repo="photonstorm/phaser-examples",
        ref="master",
        paths=("examples/assets/games",),
        category="pixel_art",
        license="Mixed; Phaser example assets",
        attribution="Phaser example game art",
        notes="Game art from the Phaser examples.",
        limit=200,
        accept="pixel_art",
    ),
    Source(
        id="phaser-example-rox",
        repo="photonstorm/phaser-examples",
        ref="master",
        paths=("projects/rox/prep",),
        category="pixel_art",
        license="Mixed; Phaser example assets",
        attribution="Phaser Rox project art",
        notes="Prepared art from the Phaser Rox project.",
        limit=200,
        accept="pixel_art",
    ),
    Source(
        id="godot-2d-tiles",
        repo="godotengine/godot-demo-projects",
        ref="master",
        paths=(
            "2d/hexagonal_map/tiles",
            "2d/isometric/decorations",
            "2d/platformer/level",
            "2d/role_playing_game/grid_movement",
        ),
        category="pixel_art",
        license="MIT for the repository; the demo assets carry their own terms",
        attribution="Godot demo project 2D art",
        notes="Tiles and decorations from the Godot 2D demos.",
        limit=200,
        accept="pixel_art",
    ),
)


def order_key(path: str) -> str:
    """The deterministic order candidates are taken in."""
    return hashlib.sha256(path.encode("utf-8")).hexdigest()


class GitHubError(RuntimeError):
    """A GitHub request that did not succeed."""


class GitHub:
    """A small GitHub client: resolve a ref, list a tree, fetch a blob.

    The token is optional. An unauthenticated client may list the trees and
    fetch a few hundred files; a token raises both limits. The token is read
    from the environment and is never written to the manifest.
    """

    def __init__(self, token: str | None = None, retries: int = 4, timeout: int = 60):
        # The token is read from the environment and is never written to the
        # manifest or to a command line, where it would be visible in the
        # process list.
        self.token = token or os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
        self.retries = retries
        self.timeout = timeout
        self.requests = 0
        self.downloaded_bytes = 0

    # -- plumbing ------------------------------------------------------------
    def _headers(self, raw: bool = False) -> dict[str, str]:
        headers = {"User-Agent": USER_AGENT}
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        if raw:
            headers["Accept"] = "application/vnd.github.raw"
        return headers

    def _fetch(self, url: str, raw: bool = False) -> bytes:
        last: Exception | None = None
        for attempt in range(self.retries):
            try:
                request = urllib.request.Request(url, headers=self._headers(raw))
                with urllib.request.urlopen(request, timeout=self.timeout) as response:
                    data = response.read()
                self.requests += 1
                return data
            except urllib.error.HTTPError as exc:
                last = exc
                if exc.code not in (403, 429, 500, 502, 503, 504):
                    raise GitHubError(f"{url}: HTTP {exc.code}") from exc
            except (urllib.error.URLError, TimeoutError, OSError) as exc:
                last = exc
            time.sleep(1.5 * (attempt + 1))
        raise GitHubError(f"{url}: {last}")

    # -- the API -------------------------------------------------------------
    def resolve_commit(self, repo: str, ref: str) -> str:
        """The commit SHA a ref points at right now."""
        url = f"{API_ROOT}/repos/{repo}/commits/{ref}"
        data = json.loads(self._fetch(url))
        return data["sha"]

    def list_tree(self, repo: str, commit: str, paths: tuple[str, ...]) -> list[dict]:
        """Every blob in the commit under `paths`, as tree entries.

        The recursive tree listing is one request. When GitHub truncates it
        (very large repositories), the wanted directories are walked with the
        contents API instead, which is more requests but bounded.
        """
        url = f"{API_ROOT}/repos/{repo}/git/trees/{commit}?recursive=1"
        data = json.loads(self._fetch(url))
        entries = data.get("tree", [])
        if not data.get("truncated"):
            return [entry for entry in entries if entry.get("type") == "blob"]
        return self._walk_contents(repo, commit, paths)

    def _walk_contents(self, repo: str, commit: str, paths: tuple[str, ...]) -> list[dict]:
        found: list[dict] = []
        stack = list(paths)
        while stack:
            current = stack.pop()
            url = f"{API_ROOT}/repos/{repo}/contents/{current}?ref={commit}"
            listing = json.loads(self._fetch(url))
            if isinstance(listing, dict):
                if listing.get("type") == "file":
                    found.append(
                        {
                            "path": listing["path"],
                            "type": "blob",
                            "size": listing.get("size", 0),
                        }
                    )
                continue
            for item in listing:
                if item.get("type") == "dir":
                    stack.append(item["path"])
                elif item.get("type") == "file":
                    found.append(
                        {
                            "path": item["path"],
                            "type": "blob",
                            "size": item.get("size", 0),
                        }
                    )
        return found

    def download(self, repo: str, commit: str, path: str) -> bytes:
        """The bytes of one file at one commit."""
        url = f"{RAW_ROOT}/{repo}/{commit}/{urllib.parse.quote(path)}"
        data = self._fetch(url, raw=True)
        self.downloaded_bytes += len(data)
        return data
