// The window's geometry.
//
// These tests exercise the arithmetic in `ui/geometry.js` on its own. The
// module is pure — it touches no DOM, no canvas and no network — so it can
// be tested in Node without a browser.
//
// What is pinned here is the behaviour the *engine* depends on: the region
// the window asks for must be inside the picture, must cover the canvas,
// and must not be larger than the engine's limit. A mistake in this file
// shows up as a request the engine refuses, or as a picture that is drawn
// in the wrong place.

import test from "node:test";
import assert from "node:assert/strict";

import {
  MAX_REGION_PIXELS,
  MAX_SCALE,
  MIN_SCALE,
  actualSizeView,
  clamp,
  clampOffset,
  clampScale,
  createView,
  dragRectangle,
  fitScale,
  fitView,
  imagePointAt,
  imageToScreen,
  panBy,
  scaleBounds,
  screenToImage,
  visibleRect,
  zoomAt,
} from "../geometry.js";

test("clamp keeps a value between its bounds", () => {
  assert.equal(clamp(5, 0, 10), 5);
  assert.equal(clamp(-1, 0, 10), 0);
  assert.equal(clamp(11, 0, 10), 10);
  assert.equal(clamp(5, 10, 0), 10, "an inverted range yields the low bound");
});

test("a new view starts at one-to-one", () => {
  const view = createView(100, 50);
  assert.equal(view.scale, 1);
  assert.equal(view.imageWidth, 100);
  assert.equal(view.imageHeight, 50);
});

test("the fit scale is the smaller of the two ratios", () => {
  assert.equal(fitScale(1000, 500, 100, 100), 0.1);
  assert.equal(fitScale(500, 1000, 100, 100), 0.1);
  assert.equal(fitScale(100, 100, 100, 100), 1);
  assert.equal(fitScale(0, 0, 100, 100), 1, "an empty picture has no fit scale");
});

test("the scale's lower bound follows the region limit", () => {
  const view = createView(4000, 4000);
  const bounds = scaleBounds(view, 1000, 1000, 1000);
  // A thousand pixels of limit over a million pixels of canvas means a
  // visible area of at most a thousand pixels, so the scale cannot be below
  // one thirty-secondth.
  assert.ok(bounds.min > 0.03, `the bound was ${bounds.min}`);
  assert.equal(bounds.max, MAX_SCALE);

  // A limit so generous that it does not bind leaves the window's own floor
  // as the lower bound.
  const roomy = scaleBounds(view, 1000, 1000, 1e12);
  assert.equal(roomy.min, MIN_SCALE);
});

test("a zoom is clamped to the bounds", () => {
  const bounds = { min: 0.5, max: 4 };
  assert.equal(clampScale(1, bounds), 1);
  assert.equal(clampScale(0.1, bounds), 0.5);
  assert.equal(clampScale(10, bounds), 4);
});

test("a picture smaller than the canvas is centred", () => {
  const view = createView(100, 100);
  view.scale = 1;
  clampOffset(view, 500, 400);
  assert.equal(view.offsetX, 200);
  assert.equal(view.offsetY, 150);
});

test("a picture larger than the canvas may not be dragged past its edge", () => {
  const view = createView(1000, 1000);
  view.scale = 1;
  view.offsetX = -500;
  view.offsetY = 50;
  clampOffset(view, 500, 500);
  assert.equal(view.offsetX, -500, "the left edge is at the canvas's left edge");
  assert.equal(view.offsetY, 0, "the top edge may not go below the canvas's top");

  view.offsetX = 100;
  clampOffset(view, 500, 500);
  assert.equal(view.offsetX, 0);
});

test("an image point and a screen point are inverses", () => {
  const view = createView(800, 600);
  view.scale = 2.5;
  view.offsetX = -30;
  view.offsetY = 17;
  const screen = imageToScreen(view, 100, 200);
  assert.equal(screen.x, -30 + 100 * 2.5);
  assert.equal(screen.y, 17 + 200 * 2.5);
  const image = imagePointAt(view, screen.x, screen.y);
  assert.ok(Math.abs(image.x - 100) < 1e-9);
  assert.ok(Math.abs(image.y - 200) < 1e-9);
});

test("a screen point names the pixel under it", () => {
  const view = createView(800, 600);
  view.scale = 1;
  view.offsetX = 0;
  view.offsetY = 0;
  assert.deepEqual(screenToImage(view, 0, 0), { x: 0, y: 0 });
  assert.deepEqual(screenToImage(view, 10.9, 3.2), { x: 10, y: 3 });
  assert.deepEqual(screenToImage(view, -1, -1), { x: -1, y: -1 });
});

test("the visible rectangle is inside the picture", () => {
  const view = createView(200, 100);
  view.scale = 1;
  view.offsetX = 0;
  view.offsetY = 0;
  const rect = visibleRect(view, 50, 40);
  assert.deepEqual(rect, { x: 0, y: 0, width: 50, height: 40 });

  // Panned away: the rectangle follows.
  view.offsetX = -80;
  view.offsetY = -60;
  const panned = visibleRect(view, 50, 40);
  assert.deepEqual(panned, { x: 80, y: 60, width: 50, height: 40 });
  assert.ok(panned.x + panned.width <= view.imageWidth);
  assert.ok(panned.y + panned.height <= view.imageHeight);
});

test("the visible rectangle never leaves the picture", () => {
  const view = createView(100, 100);
  view.scale = 1;
  view.offsetX = 60;
  view.offsetY = 60;
  const rect = visibleRect(view, 200, 200);
  assert.equal(rect.x, 0);
  assert.equal(rect.y, 0);
  assert.equal(rect.width, 100);
  assert.equal(rect.height, 100);
});

test("a zoom keeps the point under the pointer where it is", () => {
  const view = createView(1000, 1000);
  view.scale = 1;
  view.offsetX = 0;
  view.offsetY = 0;
  const before = imagePointAt(view, 300, 200);
  zoomAt(view, 2, 300, 200, 800, 600, MAX_REGION_PIXELS);
  const after = imagePointAt(view, 300, 200);
  assert.ok(Math.abs(before.x - after.x) < 1e-9, `${before.x} became ${after.x}`);
  assert.ok(Math.abs(before.y - after.y) < 1e-9, `${before.y} became ${after.y}`);
  assert.equal(view.scale, 2);
});

test("a zoom is stopped by the region limit", () => {
  const view = createView(4000, 4000);
  view.scale = 1;
  const bounds = scaleBounds(view, 1000, 1000, 1000);
  zoomAt(view, 0.01, 500, 500, 1000, 1000, 1000);
  assert.ok(view.scale >= bounds.min, `the scale fell to ${view.scale}`);
});

test("a zoom cannot pass the window's own floor", () => {
  const view = createView(4000, 4000);
  view.scale = 1;
  for (let step = 0; step < 50; step += 1) {
    zoomAt(view, 0.5, 500, 500, 1000, 1000, MAX_REGION_PIXELS);
  }
  assert.ok(view.scale >= MIN_SCALE, `the scale fell to ${view.scale}`);
});

test("fit shows the whole picture", () => {
  const view = createView(2000, 1000);
  fitView(view, 400, 400, MAX_REGION_PIXELS);
  const rect = visibleRect(view, 400, 400);
  assert.equal(rect.width, 2000);
  assert.equal(rect.height, 1000);
});

test("fit respects the region limit", () => {
  const view = createView(4000, 4000);
  fitView(view, 1000, 1000, 1000);
  const rect = visibleRect(view, 1000, 1000);
  assert.ok(
    rect.width * rect.height <= 1000 * 1.1,
    `the fitted rectangle held ${rect.width * rect.height} pixels`,
  );
});

test("one-to-one puts the picture at its own size", () => {
  const view = createView(200, 100);
  view.scale = 4;
  actualSizeView(view, 800, 600);
  assert.equal(view.scale, 1);
  const rect = visibleRect(view, 800, 600);
  assert.equal(rect.width, 200);
  assert.equal(rect.height, 100);
});

test("a pan moves the view by screen pixels", () => {
  const view = createView(1000, 1000);
  view.scale = 1;
  panBy(view, 25, -10);
  assert.equal(view.offsetX, 25);
  assert.equal(view.offsetY, -10);
});

test("a drag's rectangle covers both corners", () => {
  const view = createView(100, 100);
  view.scale = 1;
  view.offsetX = 0;
  view.offsetY = 0;
  assert.deepEqual(dragRectangle(view, 10, 20, 30, 40), {
    x: 10,
    y: 20,
    width: 20,
    height: 20,
  });
});

test("a drag's rectangle is the same whichever way it goes", () => {
  const view = createView(100, 100);
  view.scale = 1;
  const forwards = dragRectangle(view, 10, 20, 30, 40);
  const backwards = dragRectangle(view, 30, 40, 10, 20);
  assert.deepEqual(forwards, backwards);
});

test("a drag's rectangle is clipped to the picture", () => {
  const view = createView(100, 100);
  view.scale = 1;
  const rect = dragRectangle(view, 80, 80, 200, 200);
  assert.equal(rect.x, 80);
  assert.equal(rect.width, 20);
  assert.equal(rect.height, 20);
});

test("a drag outside the picture is empty", () => {
  const view = createView(100, 100);
  view.scale = 1;
  view.offsetX = 0;
  view.offsetY = 0;
  const rect = dragRectangle(view, 200, 200, 300, 300);
  assert.equal(rect.width, 0);
  assert.equal(rect.height, 0);
});

test("the visible rectangle is never larger than the limit", () => {
  // The property the engine relies on: whatever the window asks for, the
  // engine can serve it.
  const sizes = [
    [4000, 3000],
    [640, 480],
    [16384, 8640],
  ];
  for (const [width, height] of sizes) {
    for (const limit of [1000, 10_000, 1_000_000]) {
      const view = createView(width, height);
      fitView(view, 1200, 800, limit);
      const rect = visibleRect(view, 1200, 800);
      assert.ok(
        rect.width * rect.height <= limit * 1.2,
        `${width}x${height} at limit ${limit} asked for ${rect.width * rect.height}`,
      );
    }
  }
});
