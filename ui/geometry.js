// The view's geometry: the pure arithmetic behind the window.
//
// Every function in this file is a function of its arguments and nothing
// else. It touches no DOM, no canvas and no network, so it can be tested in
// Node without a browser — and the tests in `ui/test/geometry.test.mjs` do
// exactly that.
//
// # The model
//
// The view is four numbers:
//
//   * `scale` — screen pixels per image pixel;
//   * `offsetX`, `offsetY` — the image coordinate that sits at the canvas's
//     top-left corner;
//   * `imageWidth`, `imageHeight` — the picture's size.
//
// The image point `(ix, iy)` is drawn at the screen point
// `(offsetX + ix * scale, offsetY + iy * scale)`. Everything else follows
// from that one rule.
//
// # The region limit
//
// The window asks the engine for the rectangle it can see, and the engine
// refuses a rectangle larger than its limit. The limit is on the *number of
// pixels*, so the smallest scale the window may use follows from the
// viewport's area: the visible rectangle's area grows with the square of
// one over the scale.

/// The smallest scale the window offers: the picture at 1/64 of its size.
export const MIN_SCALE = 1 / 64;

/// The largest scale the window offers: sixteen image pixels per screen pixel.
export const MAX_SCALE = 16;

/// The number of pixels one display request may carry, by default.
export const MAX_REGION_PIXELS = 4194304;

/// Clamp a value between two bounds.
export function clamp(value, low, high) {
  if (high < low) return low;
  if (value < low) return low;
  if (value > high) return high;
  return value;
}

/// A view over a picture of the given size.
export function createView(imageWidth, imageHeight) {
  return {
    imageWidth,
    imageHeight,
    scale: 1,
    offsetX: 0,
    offsetY: 0,
  };
}

/// The scale at which the whole picture fits in the viewport.
export function fitScale(imageWidth, imageHeight, viewportWidth, viewportHeight) {
  if (imageWidth <= 0 || imageHeight <= 0) return 1;
  if (viewportWidth <= 0 || viewportHeight <= 0) return 1;
  return Math.min(viewportWidth / imageWidth, viewportHeight / imageHeight);
}

/// The smallest and largest scale the window may use.
///
/// The lower bound is the larger of the window's own floor and the scale at
/// which the visible rectangle would hold more pixels than the engine
/// accepts. The two per cent of slack covers the one-pixel rounding at each
/// edge of the rectangle.
export function scaleBounds(view, viewportWidth, viewportHeight, maxPixels = MAX_REGION_PIXELS) {
  const area = Math.max(1, viewportWidth * viewportHeight);
  const needed = Math.sqrt(area / Math.max(1, maxPixels)) * 1.02;
  return {
    min: Math.max(MIN_SCALE, needed),
    max: MAX_SCALE,
  };
}

/// Clamp a scale to the bounds.
export function clampScale(scale, bounds) {
  return clamp(scale, bounds.min, bounds.max);
}

/// Keep the picture from leaving the canvas.
///
/// A picture smaller than the canvas at this scale is centred; a larger one
/// may not be dragged past the canvas's edge.
export function clampOffset(view, viewportWidth, viewportHeight) {
  const width = view.imageWidth * view.scale;
  const height = view.imageHeight * view.scale;
  if (width <= viewportWidth) {
    view.offsetX = (viewportWidth - width) / 2;
  } else {
    view.offsetX = clamp(view.offsetX, viewportWidth - width, 0);
  }
  if (height <= viewportHeight) {
    view.offsetY = (viewportHeight - height) / 2;
  } else {
    view.offsetY = clamp(view.offsetY, viewportHeight - height, 0);
  }
  return view;
}

/// The image point under a screen point, with its fraction.
export function imagePointAt(view, screenX, screenY) {
  return {
    x: (screenX - view.offsetX) / view.scale,
    y: (screenY - view.offsetY) / view.scale,
  };
}

/// The image pixel under a screen point.
export function screenToImage(view, screenX, screenY) {
  const point = imagePointAt(view, screenX, screenY);
  return { x: Math.floor(point.x), y: Math.floor(point.y) };
}

/// The screen point an image pixel is drawn at.
export function imageToScreen(view, imageX, imageY) {
  return {
    x: view.offsetX + imageX * view.scale,
    y: view.offsetY + imageY * view.scale,
  };
}

/// The rectangle of the picture the canvas can see, clipped to the picture.
export function visibleRect(view, viewportWidth, viewportHeight) {
  const left = Math.max(0, Math.floor(-view.offsetX / view.scale));
  const top = Math.max(0, Math.floor(-view.offsetY / view.scale));
  const right = Math.min(
    view.imageWidth,
    Math.ceil((viewportWidth - view.offsetX) / view.scale),
  );
  const bottom = Math.min(
    view.imageHeight,
    Math.ceil((viewportHeight - view.offsetY) / view.scale),
  );
  return {
    x: left,
    y: top,
    width: Math.max(0, right - left),
    height: Math.max(0, bottom - top),
  };
}

/// Move the view by a number of screen pixels.
export function panBy(view, dx, dy) {
  view.offsetX += dx;
  view.offsetY += dy;
  return view;
}

/// Zoom by a factor, keeping the image point under the pointer where it is.
export function zoomAt(
  view,
  factor,
  screenX,
  screenY,
  viewportWidth,
  viewportHeight,
  maxPixels = MAX_REGION_PIXELS,
) {
  const bounds = scaleBounds(view, viewportWidth, viewportHeight, maxPixels);
  const before = imagePointAt(view, screenX, screenY);
  view.scale = clampScale(view.scale * factor, bounds);
  const after = imagePointAt(view, screenX, screenY);
  view.offsetX += (after.x - before.x) * view.scale;
  view.offsetY += (after.y - before.y) * view.scale;
  clampOffset(view, viewportWidth, viewportHeight);
  return view;
}

/// Put the whole picture in the canvas, at the smallest scale the limit allows.
export function fitView(view, viewportWidth, viewportHeight, maxPixels = MAX_REGION_PIXELS) {
  const bounds = scaleBounds(view, viewportWidth, viewportHeight, maxPixels);
  view.scale = clampScale(fitScale(view.imageWidth, view.imageHeight, viewportWidth, viewportHeight), bounds);
  clampOffset(view, viewportWidth, viewportHeight);
  return view;
}

/// Put the picture at one image pixel per screen pixel, centred.
export function actualSizeView(view, viewportWidth, viewportHeight) {
  view.scale = 1;
  clampOffset(view, viewportWidth, viewportHeight);
  return view;
}

/// The rectangle a drag from one screen point to another covers, in image
/// pixels, clipped to the picture.
///
/// The rectangle is the smallest one that holds both corners, so a drag to
/// the left and up still selects a rectangle.
export function dragRectangle(view, fromX, fromY, toX, toY) {
  const start = imagePointAt(view, fromX, fromY);
  const end = imagePointAt(view, toX, toY);
  const left = Math.max(0, Math.floor(Math.min(start.x, end.x)));
  const top = Math.max(0, Math.floor(Math.min(start.y, end.y)));
  const right = Math.min(view.imageWidth, Math.ceil(Math.max(start.x, end.x)));
  const bottom = Math.min(view.imageHeight, Math.ceil(Math.max(start.y, end.y)));
  return {
    x: left,
    y: top,
    width: Math.max(0, right - left),
    height: Math.max(0, bottom - top),
  };
}
