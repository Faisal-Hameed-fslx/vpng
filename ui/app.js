// The window.
//
// This file is the wiring: it talks to the engine over the protocol in
// `crates/vpng-ui/src/protocol.rs`, draws what the engine sends on a canvas,
// and turns the pointer and the keyboard into commands.
//
// The arithmetic — where a screen point is in the picture, which rectangle a
// drag covers, how far the view may zoom out — lives in `geometry.js`, which
// is pure and is tested on its own.

import {
  MAX_REGION_PIXELS,
  actualSizeView,
  clampOffset,
  createView,
  dragRectangle,
  fitView,
  imageToScreen,
  screenToImage,
  visibleRect,
  zoomAt,
} from "./geometry.js";

const parameters = new URLSearchParams(location.search);
const token = parameters.get("token") || "";

const canvas = document.getElementById("canvas");
const context = canvas.getContext("2d");
const overlay = document.getElementById("overlay");
const banner = document.getElementById("banner");

const elements = {
  file: document.getElementById("file"),
  zoom: document.getElementById("zoom"),
  alpha: document.getElementById("alpha"),
  alphaValue: document.getElementById("alpha-value"),
  colour: document.getElementById("colour"),
  undo: document.getElementById("undo"),
  redo: document.getElementById("redo"),
  save: document.getElementById("save"),
  fit: document.getElementById("fit"),
  actual: document.getElementById("actual"),
  size: document.getElementById("status-size"),
  format: document.getElementById("status-format"),
  cursor: document.getElementById("status-cursor"),
  pixel: document.getElementById("status-pixel"),
  tiles: document.getElementById("status-tiles"),
  memory: document.getElementById("status-memory"),
  dirty: document.getElementById("status-dirty"),
};

let info = null;
let view = createView(1, 1);
let tool = "pan";
let colour = { r: 0, g: 0, b: 0, a: 255 };
let drag = null;
let drawing = false;
let queued = false;

/// The size of the canvas in CSS pixels.
function viewport() {
  return {
    width: canvas.clientWidth,
    height: canvas.clientHeight,
  };
}

/// The engine's region limit.
function regionLimit() {
  return info ? info.max_region_pixels : MAX_REGION_PIXELS;
}

/// Send a command to the engine.
async function api(path, body) {
  const options = {
    method: body === undefined ? "GET" : "POST",
    headers: { "X-Vpng-Token": token },
  };
  if (body !== undefined) {
    options.headers["Content-Type"] = "application/json";
    options.body = JSON.stringify(body);
  }
  const response = await fetch(path, options);
  const text = await response.text();
  let payload = null;
  try {
    payload = JSON.parse(text);
  } catch (error) {
    payload = null;
  }
  if (!response.ok) {
    const message =
      payload && payload.error ? payload.error.message : `the engine answered ${response.status}`;
    throw new Error(message);
  }
  return payload;
}

/// Show a message in the banner.
function report(message, kind) {
  if (!message) {
    banner.hidden = true;
    banner.textContent = "";
    banner.className = "banner";
    return;
  }
  banner.hidden = false;
  banner.textContent = message;
  banner.className = kind === "info" ? "banner info" : "banner";
}

/// Read the engine's state.
async function refresh() {
  info = await api("/api/state");
  view.imageWidth = info.width;
  view.imageHeight = info.height;
  tool = info.tool;
  colour = info.colour;
  elements.alpha.value = String(colour.a);
  elements.alphaValue.textContent = String(colour.a);
  elements.colour.value = rgbToHex(colour);
  updateToolbar();
}

/// Update the toolbar and the status bar from the engine's state.
function updateToolbar() {
  if (!info) return;
  elements.file.textContent = info.path;
  elements.undo.disabled = !info.can_undo;
  elements.redo.disabled = !info.can_redo;
  elements.save.disabled = !info.dirty;
  elements.size.textContent = `${info.width} x ${info.height}`;
  elements.format.textContent = `${info.format}`;
  elements.tiles.textContent = `${info.tile_count} tiles, ${info.visible_tiles} visible, ${info.prefetch_tiles} prefetch`;
  elements.memory.textContent = `cache ${formatBytes(info.cache_current_bytes)}, history ${formatBytes(info.history_current_bytes)}`;
  elements.dirty.textContent = info.dirty
    ? `${info.dirty_tiles} dirty tiles, ${info.edits} edits`
    : "saved";
  elements.dirty.className = info.dirty ? "mono dirty" : "mono";
  for (const button of document.querySelectorAll(".tool")) {
    button.classList.toggle("active", button.dataset.tool === tool);
  }
}

/// A number of bytes, in the largest unit that keeps it readable.
function formatBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

/// A colour as the colour input wants it.
function rgbToHex(value) {
  const part = (channel) => channel.toString(16).padStart(2, "0");
  return `#${part(value.r)}${part(value.g)}${part(value.b)}`;
}

/// Draw the visible part of the picture.
async function draw() {
  if (drawing) {
    queued = true;
    return;
  }
  drawing = true;
  try {
    do {
      queued = false;
      await drawOnce();
    } while (queued);
  } catch (error) {
    report(error.message);
  } finally {
    drawing = false;
  }
}

/// One pass of the drawing.
async function drawOnce() {
  const size = viewport();
  if (size.width === 0 || size.height === 0) return;

  const ratio = window.devicePixelRatio || 1;
  const width = Math.round(size.width * ratio);
  const height = Math.round(size.height * ratio);
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.clearRect(0, 0, size.width, size.height);

  const rect = visibleRect(view, size.width, size.height);
  if (rect.width === 0 || rect.height === 0) return;

  const response = await fetch("/api/region", {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Vpng-Token": token },
    body: JSON.stringify(rect),
  });
  if (!response.ok) {
    const text = await response.text();
    let message = text;
    try {
      const payload = JSON.parse(text);
      if (payload && payload.error) message = payload.error.message;
    } catch (error) {
      // The body was not JSON; the text is the message.
    }
    throw new Error(message);
  }

  const buffer = await response.arrayBuffer();
  const pixels = new Uint8ClampedArray(buffer);
  if (pixels.length !== rect.width * rect.height * 4) {
    throw new Error(
      `the engine sent ${pixels.length} bytes for a ${rect.width}x${rect.height} region`,
    );
  }

  const scratch = document.createElement("canvas");
  scratch.width = rect.width;
  scratch.height = rect.height;
  scratch.getContext("2d").putImageData(new ImageData(pixels, rect.width, rect.height), 0, 0);

  context.imageSmoothingEnabled = false;
  const origin = imageToScreen(view, rect.x, rect.y);
  context.drawImage(
    scratch,
    origin.x,
    origin.y,
    rect.width * view.scale,
    rect.height * view.scale,
  );

  elements.zoom.textContent = `${Math.round(view.scale * 100)}%`;
}

/// The picture's coordinate under a pointer event.
function pointerImage(event) {
  const bounds = canvas.getBoundingClientRect();
  return screenToImage(view, event.clientX - bounds.left, event.clientY - bounds.top);
}

/// The pointer's position, for the status bar.
function reportCursor(event) {
  const bounds = canvas.getBoundingClientRect();
  const point = screenToImage(view, event.clientX - bounds.left, event.clientY - bounds.top);
  if (point.x < 0 || point.y < 0 || point.x >= view.imageWidth || point.y >= view.imageHeight) {
    elements.cursor.textContent = "—";
    elements.pixel.textContent = "";
    return;
  }
  elements.cursor.textContent = `${point.x}, ${point.y}`;
}

/// Read the pixel under the pointer.
async function pick(point) {
  try {
    const pixel = await api("/api/pick", { x: point.x, y: point.y });
    elements.pixel.textContent = `rgba(${pixel.r}, ${pixel.g}, ${pixel.b}, ${pixel.a})`;
    colour = { r: pixel.r, g: pixel.g, b: pixel.b, a: pixel.a };
    elements.colour.value = rgbToHex(colour);
    elements.alpha.value = String(colour.a);
    elements.alphaValue.textContent = String(colour.a);
  } catch (error) {
    report(error.message);
  }
}

/// Apply an edit and show what it did.
async function edit(path, body) {
  try {
    const result = await api(path, body);
    report(`the edit changed ${result.pixels} pixels; ${result.dirty_tiles} tiles are dirty`, "info");
    await refresh();
    await draw();
  } catch (error) {
    report(error.message);
  }
}

/// Choose a tool.
async function chooseTool(name) {
  tool = name;
  try {
    await api("/api/tool", { tool: name });
    await refresh();
  } catch (error) {
    report(error.message);
  }
}

/// Set the colour.
async function setColour() {
  const hex = elements.colour.value;
  colour.r = parseInt(hex.slice(1, 3), 16);
  colour.g = parseInt(hex.slice(3, 5), 16);
  colour.b = parseInt(hex.slice(5, 7), 16);
  colour.a = Number(elements.alpha.value);
  try {
    await api("/api/colour", colour);
    await refresh();
  } catch (error) {
    report(error.message);
  }
}

// The pointer.

canvas.addEventListener("pointerdown", (event) => {
  canvas.setPointerCapture(event.pointerId);
  const size = viewport();
  const bounds = canvas.getBoundingClientRect();
  const x = event.clientX - bounds.left;
  const y = event.clientY - bounds.top;

  if (tool === "pan" || event.button === 1) {
    drag = { kind: "pan", x, y };
    return;
  }
  if (tool === "pick") {
    drag = { kind: "pick" };
    pick(pointerImage(event));
    return;
  }
  drag = { kind: "rect", x, y };
});

canvas.addEventListener("pointermove", (event) => {
  reportCursor(event);
  if (!drag) return;
  const size = viewport();
  const bounds = canvas.getBoundingClientRect();
  const x = event.clientX - bounds.left;
  const y = event.clientY - bounds.top;

  if (drag.kind === "pan") {
    const dx = x - drag.x;
    const dy = y - drag.y;
    drag.x = x;
    drag.y = y;
    view.offsetX += dx;
    view.offsetY += dy;
    clampOffset(view, size.width, size.height);
    draw();
    return;
  }
  if (drag.kind === "rect") {
    const rectangle = dragRectangle(view, drag.x, drag.y, x, y);
    overlay.hidden = false;
    overlay.style.left = `${imageToScreen(view, rectangle.x, rectangle.y).x}px`;
    overlay.style.top = `${imageToScreen(view, rectangle.x, rectangle.y).y}px`;
    overlay.style.width = `${rectangle.width * view.scale}px`;
    overlay.style.height = `${rectangle.height * view.scale}px`;
  }
});

canvas.addEventListener("pointerup", async (event) => {
  const size = viewport();
  const bounds = canvas.getBoundingClientRect();
  const x = event.clientX - bounds.left;
  const y = event.clientY - bounds.top;
  const current = drag;
  drag = null;
  overlay.hidden = true;
  if (!current) return;

  if (current.kind === "rect") {
    const rectangle = dragRectangle(view, current.x, current.y, x, y);
    if (rectangle.width === 0 || rectangle.height === 0) return;
    if (tool === "fill") {
      await edit("/api/fill", rectangle);
    } else if (tool === "alpha") {
      await edit("/api/alpha", { ...rectangle, value: colour.a });
    }
  }
  if (current.kind === "pan") {
    await draw();
  }
});

canvas.addEventListener("pointerleave", () => {
  elements.cursor.textContent = "—";
  elements.pixel.textContent = "";
});

canvas.addEventListener("wheel", (event) => {
  event.preventDefault();
  const size = viewport();
  const bounds = canvas.getBoundingClientRect();
  const factor = event.deltaY < 0 ? 1.2 : 1 / 1.2;
  zoomAt(
    view,
    factor,
    event.clientX - bounds.left,
    event.clientY - bounds.top,
    size.width,
    size.height,
    regionLimit(),
  );
  draw();
}, { passive: false });

// The toolbar.

for (const button of document.querySelectorAll(".tool")) {
  button.addEventListener("click", () => chooseTool(button.dataset.tool));
}

elements.colour.addEventListener("change", setColour);
elements.alpha.addEventListener("input", () => {
  elements.alphaValue.textContent = elements.alpha.value;
});
elements.alpha.addEventListener("change", setColour);

elements.undo.addEventListener("click", async () => {
  await edit("/api/undo", {});
});

elements.redo.addEventListener("click", async () => {
  await edit("/api/redo", {});
});

elements.save.addEventListener("click", async () => {
  try {
    const saved = await api("/api/save", {});
    report(
      `saved ${saved.file_size} bytes; ${saved.tiles_rewritten} tiles rewritten, ${saved.tiles_copied} copied`,
      "info",
    );
    await refresh();
    await draw();
  } catch (error) {
    report(error.message);
  }
});

elements.fit.addEventListener("click", async () => {
  const size = viewport();
  fitView(view, size.width, size.height, regionLimit());
  await draw();
});

elements.actual.addEventListener("click", async () => {
  const size = viewport();
  actualSizeView(view, size.width, size.height);
  await draw();
});

// The keyboard.

window.addEventListener("keydown", async (event) => {
  if (event.ctrlKey || event.metaKey) {
    if (event.key === "z" && !event.shiftKey) {
      event.preventDefault();
      await api("/api/undo", {}).catch((error) => report(error.message));
      await refresh();
      await draw();
      return;
    }
    if (event.key === "y" || (event.key === "z" && event.shiftKey)) {
      event.preventDefault();
      await api("/api/redo", {}).catch((error) => report(error.message));
      await refresh();
      await draw();
      return;
    }
    if (event.key === "s") {
      event.preventDefault();
      elements.save.click();
      return;
    }
    return;
  }
  const shortcuts = { v: "pan", i: "pick", f: "fill", a: "alpha" };
  const chosen = shortcuts[event.key.toLowerCase()];
  if (chosen) {
    event.preventDefault();
    await chooseTool(chosen);
  }
});

window.addEventListener("resize", () => {
  const size = viewport();
  clampOffset(view, size.width, size.height);
  draw();
});

// Start.

async function start() {
  try {
    await refresh();
    const size = viewport();
    if (size.width > 0 && size.height > 0) {
      fitView(view, size.width, size.height, regionLimit());
    }
    await draw();
    report(
      "The editor shows the picture's original level. A file with a preview pyramid is not used for zoom-out in this build.",
      "info",
    );
  } catch (error) {
    report(error.message);
  }
}

start();
