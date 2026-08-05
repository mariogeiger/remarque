import init, { BrowserPageRenderer } from "__REMARQUE_BROWSER_RENDERER_MODULE__";

const canvas = document.querySelector("#page");
const context = canvas.getContext("2d", { alpha: false });
const status = document.querySelector("#status");
const eraser = document.querySelector("#eraser");
const thicknessButtons = [...document.querySelectorAll("[data-thickness]")];
let renderer;
let socket;
let pendingSamples = [];
let appendScheduled = false;
let erase = false;
let eraserPoints = [];
let eraserPreviewScheduled = false;
let loadedBackground;
let activeShare;
let participantSessionToken;
let reconnectDelay = 1000;
let stopped = false;
let activePointer;
let canvasImage;

function parseCapability() {
  const capability = location.hash.slice(1);
  const separator = capability.indexOf(".");
  if (separator >= 1) {
    return { share: capability.slice(0, separator), secret: capability.slice(separator + 1) };
  }
  const path = location.pathname.match(/^\/share\/([0-9a-f]{32})$/);
  if (path) return { share: path[1] };
  throw new Error("Lien de partage invalide");
}

async function redeem({ share, secret }) {
  const storageKey = `remarque.participant.${share}`;
  const storedSessionToken = localStorage.getItem(storageKey) || undefined;
  if (!secret) return { share, sessionToken: storedSessionToken };
  const response = await fetch(`/api/shares/${share}/redeem`, {
    method: "POST",
    credentials: "same-origin",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ secret, session_token: storedSessionToken }),
  });
  if (!response.ok) throw new Error("Ce partage est invalide ou expiré");
  const redemption = await response.json();
  if (!/^[0-9a-f]{64}$/.test(redemption.session_token)) {
    throw new Error("Réponse de partage invalide");
  }
  localStorage.setItem(storageKey, redemption.session_token);
  return { share, sessionToken: redemption.session_token };
}

function paint() {
  const width = renderer.width();
  const height = renderer.height();
  if (!canvasImage || canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
    canvas.style.aspectRatio = `${width} / ${height}`;
    canvasImage = context.createImageData(width, height);
  }
  const pixels = renderer.rgba_pixels();
  const dirtyWidth = renderer.dirty_width();
  const dirtyHeight = renderer.dirty_height();
  if (dirtyWidth && dirtyHeight) {
    const dirtyX = renderer.dirty_x();
    const dirtyY = renderer.dirty_y();
    const rowLength = dirtyWidth * 4;
    for (let row = 0; row < dirtyHeight; row += 1) {
      const start = ((dirtyY + row) * width + dirtyX) * 4;
      canvasImage.data.set(pixels.subarray(start, start + rowLength), start);
    }
    context.putImageData(
      canvasImage,
      0,
      0,
      dirtyX,
      dirtyY,
      dirtyWidth,
      dirtyHeight,
    );
    renderer.clear_dirty_rectangle();
  }
}

function send(bytes) {
  if (socket?.readyState === WebSocket.OPEN) socket.send(bytes);
}

async function loadBackground() {
  const digest = renderer.background_digest();
  if (!digest || digest === loadedBackground) return;
  const headers = participantSessionToken
    ? { authorization: `Bearer ${participantSessionToken}` }
    : {};
  const response = await fetch(`/api/shares/${activeShare}/assets/${digest}`, {
    credentials: "same-origin",
    headers,
  });
  if (!response.ok) throw new Error("Fond de page indisponible");
  renderer.set_background_bgra(new Uint8Array(await response.arrayBuffer()));
  loadedBackground = digest;
  paint();
}

function pagePoint(event) {
  const rectangle = canvas.getBoundingClientRect();
  return [
    (event.clientX - rectangle.left) * canvas.width / rectangle.width,
    (event.clientY - rectangle.top) * canvas.height / rectangle.height,
    event.pressure || 0.5,
  ];
}

function scheduleAppend() {
  if (appendScheduled) return;
  appendScheduled = true;
  requestAnimationFrame(() => {
    appendScheduled = false;
    if (!pendingSamples.length) return;
    send(renderer.append_samples(new Float32Array(pendingSamples)));
    pendingSamples = [];
  });
}

function scheduleEraserPreview() {
  if (eraserPreviewScheduled || !eraserPoints.length) return;
  eraserPreviewScheduled = true;
  requestAnimationFrame(() => {
    eraserPreviewScheduled = false;
    if (!erase || activePointer === undefined || !eraserPoints.length) return;
    try {
      renderer.preview_erase_with_centerline(new Float64Array(eraserPoints), 30);
      paint();
    } catch (error) {
      status.textContent = error instanceof Error ? error.message : String(error);
    }
  });
}

function setEraserSelected(selected) {
  erase = selected;
  eraser.setAttribute("aria-pressed", String(selected));
}

canvas.addEventListener("pointerdown", (event) => {
  if (!renderer?.ready() || activePointer !== undefined) return;
  activePointer = event.pointerId;
  canvas.setPointerCapture(event.pointerId);
  if (erase) {
    const [x, y] = pagePoint(event);
    eraserPoints.push(x, y);
    scheduleEraserPreview();
  } else {
    send(renderer.begin_stroke());
    pendingSamples.push(...pagePoint(event));
    scheduleAppend();
  }
});

canvas.addEventListener("pointermove", (event) => {
  if (event.pointerId !== activePointer || !canvas.hasPointerCapture(event.pointerId)) return;
  for (const sample of event.getCoalescedEvents?.() || [event]) {
    const point = pagePoint(sample);
    if (erase) eraserPoints.push(point[0], point[1]);
    else pendingSamples.push(...point);
  }
  if (erase) scheduleEraserPreview();
  else scheduleAppend();
});

canvas.addEventListener("pointerup", (event) => {
  if (event.pointerId !== activePointer || !canvas.hasPointerCapture(event.pointerId)) return;
  if (erase) {
    const [x, y] = pagePoint(event);
    eraserPoints.push(x, y);
    renderer.preview_erase_with_centerline(new Float64Array(eraserPoints), 30);
    const command = renderer.erase_with_centerline(new Float64Array(eraserPoints), 30);
    if (command.length) send(command);
    paint();
    eraserPoints = [];
    canvas.releasePointerCapture(event.pointerId);
    activePointer = undefined;
    return;
  }
  pendingSamples.push(...pagePoint(event));
  if (pendingSamples.length) {
    send(renderer.append_samples(new Float32Array(pendingSamples)));
    pendingSamples = [];
  }
  send(renderer.commit_stroke());
  canvas.releasePointerCapture(event.pointerId);
  activePointer = undefined;
});

canvas.addEventListener("pointercancel", (event) => {
  if (event.pointerId !== activePointer || !canvas.hasPointerCapture(event.pointerId)) return;
  if (erase) {
    renderer.cancel_erase_preview();
    paint();
    eraserPoints = [];
    canvas.releasePointerCapture(event.pointerId);
    activePointer = undefined;
    return;
  }
  pendingSamples = [];
  send(renderer.cancel_stroke());
  canvas.releasePointerCapture(event.pointerId);
  activePointer = undefined;
});

eraser.addEventListener("click", () => {
  setEraserSelected(!erase);
});

for (const button of thicknessButtons) {
  button.addEventListener("click", () => {
    const preset = Number(button.dataset.thickness);
    renderer.set_fineliner_thickness(preset);
    setEraserSelected(false);
    for (const candidate of thicknessButtons) {
      candidate.setAttribute("aria-pressed", String(candidate === button));
    }
  });
}

async function start() {
  await init();
  renderer = new BrowserPageRenderer(crypto.getRandomValues(new Uint8Array(16)));
  const { share, sessionToken } = await redeem(parseCapability());
  activeShare = share;
  participantSessionToken = sessionToken;
  connect(share);
}

function connect(share) {
  if (stopped) return;
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  const url = `${scheme}://${location.host}/api/shares/${share}/ws`;
  const connection = participantSessionToken
    ? new WebSocket(url, `remarque.session.${participantSessionToken}`)
    : new WebSocket(url);
  socket = connection;
  connection.binaryType = "arraybuffer";
  status.textContent = "Connexion…";
  connection.addEventListener("open", () => {
    reconnectDelay = 1000;
    status.textContent = "Connecté";
  });
  connection.addEventListener("message", (event) => {
    try {
      renderer.apply_server_message(new Uint8Array(event.data));
      if (renderer.take_reconnect_ready()) {
        if (activePointer !== undefined && canvas.hasPointerCapture(activePointer)) {
          canvas.releasePointerCapture(activePointer);
        }
        activePointer = undefined;
        pendingSamples = [];
        eraserPoints = [];
        for (const bytes of renderer.pending_messages()) send(bytes);
      }
      paint();
      if (erase && activePointer !== undefined && eraserPoints.length) {
        scheduleEraserPreview();
      }
      loadBackground().catch((error) => { status.textContent = error.message; });
      status.style.borderLeft = `12px solid ${renderer.participant_color()}`;
      if (renderer.needs_snapshot()) send(renderer.request_snapshot());
      const rejection = renderer.take_rejection();
      if (rejection) {
        status.textContent = rejection;
        stopped = /revoked|expired|révoqué|expiré/i.test(rejection);
      }
    } catch (error) {
      const description = error instanceof Error
        ? error.message
        : String(error);
      status.textContent = description;
      send(renderer.request_snapshot());
    }
  });
  connection.addEventListener("close", () => {
    if (socket !== connection || stopped) return;
    status.textContent = "Déconnecté — reconnexion…";
    setTimeout(() => connect(share), reconnectDelay);
    reconnectDelay = Math.min(reconnectDelay * 2, 30000);
  });
}

start().catch((error) => { status.textContent = error.message; });
