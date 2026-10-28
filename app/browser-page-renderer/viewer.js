import init, { BrowserPageRenderer } from "__REMARQUE_BROWSER_RENDERER_MODULE__";

const canvas = document.querySelector("#page");
const context = canvas.getContext("2d", { alpha: false });
const status = document.querySelector("#status");
const eraser = document.querySelector("#eraser");
const undo = document.querySelector("#undo");
const redo = document.querySelector("#redo");
const toolPreview = document.querySelector("#tool-preview");
const thicknessButtons = [...document.querySelectorAll("[data-thickness]")];
const finelinerTools = thicknessButtons.map((button) => ({
  kind: "fineliner",
  preset: Number(button.dataset.thickness),
  width: Number(button.dataset.pageWidth),
}));
const eraserTool = { kind: "eraser", width: 30 };
let renderer;
let socket;
let loadedBackground;
let activeShare;
let participantSessionToken;
let reconnectDelay = 1000;
let stopped = false;
let selectedFinelinerTool = finelinerTools[0];
let selectedTool = selectedFinelinerTool;
let activeGesture;
let canvasImage;
const pointerObservationSession = Array.from(
  crypto.getRandomValues(new Uint8Array(8)),
  (byte) => byte.toString(16).padStart(2, "0"),
).join("");
let pointerObservationSequence = 0;
let pointerObservationFlushScheduled = false;
const pointerEventObservations = [];

function pointerCaptureIsHeld(pointerId) {
  try {
    return pointerId !== undefined && canvas.hasPointerCapture(pointerId);
  } catch {
    return false;
  }
}

function flushPointerEventObservations() {
  if (pointerObservationFlushScheduled || !activeShare) return;
  pointerObservationFlushScheduled = true;
  setTimeout(async () => {
    pointerObservationFlushScheduled = false;
    const records = pointerEventObservations.splice(0, 64);
    if (!records.length) return;
    try {
      await fetch(`/api/shares/${activeShare}/pointer-debug`, {
        method: "POST",
        credentials: "same-origin",
        headers: {
          ...(participantSessionToken
            ? { authorization: `Bearer ${participantSessionToken}` }
            : {}),
          "content-type": "application/json",
        },
        body: JSON.stringify({ records }),
        keepalive: true,
      });
    } catch {}
    if (pointerEventObservations.length) flushPointerEventObservations();
  }, 0);
}

function recordPointerEventObservation(hypothesis, action, event, gesture = activeGesture) {
  if (!activeShare) return;
  const pointerId = event?.pointerId ?? gesture?.pointerId;
  pointerEventObservations.push({
    session: pointerObservationSession,
    sequence: pointerObservationSequence,
    client_unix_milliseconds: Date.now(),
    hypothesis,
    event: event?.type || "internal",
    action,
    pointer_type: event?.pointerType || (event?.type?.startsWith("mouse") ? "mouse" : "none"),
    pointer_id: Number.isInteger(pointerId) ? pointerId : null,
    buttons: event?.buttons ?? 0,
    captured: pointerCaptureIsHeld(pointerId),
    active: activeGesture !== undefined,
    tool: gesture?.tool.kind || "none",
    stroke_started: gesture?.strokeStarted || false,
    sample_count: gesture?.samples.length || 0,
    point_count: gesture?.points.length || 0,
    socket_state: socket?.readyState ?? 3,
  });
  pointerObservationSequence += 1;
  if (pointerEventObservations.length > 64) pointerEventObservations.shift();
  flushPointerEventObservations();
}

function parseCapability() {
  const capability = location.hash.slice(1);
  const separator = capability.indexOf(".");
  if (separator >= 1) {
    return { share: capability.slice(0, separator), secret: capability.slice(separator + 1) };
  }
  const path = location.pathname.match(/^\/share\/([0-9a-f]{32})$/);
  if (path) return { share: path[1] };
  throw new Error("Invalid share link");
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
  if (!response.ok) throw new Error("This share is invalid or has expired");
  const redemption = await response.json();
  if (!/^[0-9a-f]{64}$/.test(redemption.session_token)) {
    throw new Error("Invalid share response");
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
  if (!response.ok) throw new Error("Page background is unavailable");
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

function showError(error) {
  status.textContent = error instanceof Error ? error.message : String(error);
}

function updateHistoryButtons() {
  undo.disabled = !renderer?.can_undo();
  redo.disabled = !renderer?.can_redo();
}

function positionToolPreview(event) {
  const rectangle = canvas.getBoundingClientRect();
  const insideCanvas = event.clientX >= rectangle.left
    && event.clientX <= rectangle.right
    && event.clientY >= rectangle.top
    && event.clientY <= rectangle.bottom;
  if (!insideCanvas) {
    toolPreview.hidden = true;
    return;
  }
  const tool = activeGesture?.tool || selectedTool;
  const diameter = tool.width * rectangle.width / canvas.width;
  toolPreview.dataset.tool = tool.kind;
  toolPreview.style.left = `${event.clientX}px`;
  toolPreview.style.top = `${event.clientY}px`;
  toolPreview.style.width = `${diameter}px`;
  toolPreview.style.height = `${diameter}px`;
  toolPreview.hidden = false;
}

function stopTrackingGesture(gesture) {
  if (activeGesture !== gesture) return;
  activeGesture = undefined;
  if (canvas.hasPointerCapture(gesture.pointerId)) {
    canvas.releasePointerCapture(gesture.pointerId);
  }
}

function appendStrokeSamples(gesture) {
  if (activeGesture !== gesture || !gesture.samples.length) return;
  send(renderer.append_samples(new Float32Array(gesture.samples)));
  gesture.samples = [];
}

function scheduleStrokeSampleAppend(gesture) {
  if (gesture.appendScheduled) return;
  gesture.appendScheduled = true;
  requestAnimationFrame(() => {
    gesture.appendScheduled = false;
    try {
      appendStrokeSamples(gesture);
    } catch (error) {
      cancelGesture(gesture);
      showError(error);
    }
  });
}

function previewEraserGesture(gesture) {
  if (activeGesture !== gesture || !gesture.points.length) return;
  renderer.preview_erase_with_centerline(new Float64Array(gesture.points), gesture.tool.width);
  paint();
}

function scheduleEraserPreview(gesture) {
  if (gesture.previewScheduled || !gesture.points.length) return;
  gesture.previewScheduled = true;
  requestAnimationFrame(() => {
    gesture.previewScheduled = false;
    try {
      previewEraserGesture(gesture);
    } catch (error) {
      cancelGesture(gesture);
      showError(error);
    }
  });
}

function cancelGesture(gesture) {
  if (activeGesture !== gesture) return;
  try {
    if (gesture.tool.kind === "eraser") {
      renderer.cancel_erase_preview();
      paint();
    } else if (gesture.strokeStarted) {
      send(renderer.cancel_stroke());
      recordPointerEventObservation("H2", "cancel_sent", undefined, gesture);
    }
  } catch (error) {
    showError(error);
  } finally {
    stopTrackingGesture(gesture);
  }
}

function selectTool(tool) {
  selectedTool = tool;
  eraser.setAttribute("aria-pressed", String(tool.kind === "eraser"));
  for (let index = 0; index < thicknessButtons.length; index += 1) {
    const selected = tool.kind === "fineliner" && finelinerTools[index] === tool;
    thicknessButtons[index].setAttribute("aria-pressed", String(selected));
  }
}

canvas.addEventListener("pointerdown", (event) => {
  if (!renderer?.ready() || activeGesture !== undefined) return;
  const gesture = {
    pointerId: event.pointerId,
    tool: selectedTool,
    samples: [],
    points: [],
    appendScheduled: false,
    previewScheduled: false,
    strokeStarted: false,
  };
  activeGesture = gesture;
  recordPointerEventObservation("H1", "pointerdown_received", event, gesture);
  try {
    canvas.setPointerCapture(event.pointerId);
    if (gesture.tool.kind === "eraser") {
      const [x, y] = pagePoint(event);
      gesture.points.push(x, y);
      scheduleEraserPreview(gesture);
    } else {
      const command = renderer.begin_stroke();
      gesture.strokeStarted = true;
      send(command);
      recordPointerEventObservation("H2", "begin_sent", event, gesture);
      gesture.samples.push(...pagePoint(event));
      scheduleStrokeSampleAppend(gesture);
    }
    positionToolPreview(event);
  } catch (error) {
    recordPointerEventObservation("H2", "pointerdown_failed", event, gesture);
    cancelGesture(gesture);
    showError(error);
  }
});

canvas.addEventListener("pointermove", (event) => {
  positionToolPreview(event);
  const gesture = activeGesture;
  if (!gesture || event.pointerId !== gesture.pointerId) return;
  if (event.buttons === 0) {
    recordPointerEventObservation("H1", "pointermove_without_button", event, gesture);
  }
  try {
    for (const sample of event.getCoalescedEvents?.() || [event]) {
      const point = pagePoint(sample);
      if (gesture.tool.kind === "eraser") gesture.points.push(point[0], point[1]);
      else gesture.samples.push(...point);
    }
    if (gesture.tool.kind === "eraser") scheduleEraserPreview(gesture);
    else scheduleStrokeSampleAppend(gesture);
  } catch (error) {
    cancelGesture(gesture);
    showError(error);
  }
});

canvas.addEventListener("pointerup", (event) => {
  const gesture = activeGesture;
  recordPointerEventObservation("H1", "pointerup_received", event, gesture);
  if (!gesture || event.pointerId !== gesture.pointerId) return;
  try {
    if (gesture.tool.kind === "eraser") {
      const [x, y] = pagePoint(event);
      gesture.points.push(x, y);
      previewEraserGesture(gesture);
      const command = renderer.erase_with_centerline(
        new Float64Array(gesture.points),
        gesture.tool.width,
      );
      if (command.length) send(command);
      paint();
    } else {
      gesture.samples.push(...pagePoint(event));
      appendStrokeSamples(gesture);
      send(renderer.commit_stroke());
      recordPointerEventObservation("H2", "commit_sent", event, gesture);
      gesture.strokeStarted = false;
    }
    stopTrackingGesture(gesture);
    positionToolPreview(event);
    if (event.pointerType === "touch") toolPreview.hidden = true;
  } catch (error) {
    recordPointerEventObservation("H2", "pointerup_failed", event, gesture);
    cancelGesture(gesture);
    showError(error);
  }
});

canvas.addEventListener("pointercancel", (event) => {
  const gesture = activeGesture;
  recordPointerEventObservation("H1", "pointercancel_received", event, gesture);
  if (!gesture || event.pointerId !== gesture.pointerId) return;
  cancelGesture(gesture);
  toolPreview.hidden = true;
});

canvas.addEventListener("lostpointercapture", (event) => {
  const gesture = activeGesture;
  recordPointerEventObservation("H1", "lostpointercapture_received", event, gesture);
  if (!gesture || event.pointerId !== gesture.pointerId) return;
  cancelGesture(gesture);
});

canvas.addEventListener("pointerleave", () => {
  toolPreview.hidden = true;
});

window.addEventListener("mouseup", (event) => {
  if (activeGesture) recordPointerEventObservation("H1", "mouseup_received", event);
});

canvas.addEventListener("dragstart", (event) => {
  recordPointerEventObservation("H1", "dragstart_received", event);
});

canvas.addEventListener("dragend", (event) => {
  recordPointerEventObservation("H1", "dragend_received", event);
});

eraser.addEventListener("click", () => {
  selectTool(eraserTool);
});

undo.addEventListener("click", () => {
  try {
    send(renderer.undo());
  } catch (error) {
    showError(error);
  }
});

redo.addEventListener("click", () => {
  try {
    send(renderer.redo());
  } catch (error) {
    showError(error);
  }
});

for (let index = 0; index < thicknessButtons.length; index += 1) {
  const button = thicknessButtons[index];
  button.addEventListener("click", () => {
    selectedFinelinerTool = finelinerTools[index];
    renderer?.set_fineliner_thickness(selectedFinelinerTool.preset);
    selectTool(selectedFinelinerTool);
  });
}

async function start() {
  await init();
  renderer = new BrowserPageRenderer(crypto.getRandomValues(new Uint8Array(16)));
  renderer.set_fineliner_thickness(selectedFinelinerTool.preset);
  updateHistoryButtons();
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
  status.textContent = "Connecting…";
  connection.addEventListener("open", () => {
    reconnectDelay = 1000;
    status.textContent = "Connected";
  });
  connection.addEventListener("message", (event) => {
    try {
      renderer.apply_server_message(new Uint8Array(event.data));
      if (renderer.take_reconnect_ready()) {
        recordPointerEventObservation("H3", "reconnect_ready", event);
        if (activeGesture) stopTrackingGesture(activeGesture);
        for (const bytes of renderer.pending_messages()) send(bytes);
      }
      paint();
      updateHistoryButtons();
      if (activeGesture?.tool.kind === "eraser" && activeGesture.points.length) {
        scheduleEraserPreview(activeGesture);
      }
      loadBackground().catch((error) => { status.textContent = error.message; });
      status.style.borderLeft = `12px solid ${renderer.participant_color()}`;
      toolPreview.style.setProperty("--participant-color", renderer.participant_color());
      if (renderer.needs_snapshot()) send(renderer.request_snapshot());
      const rejection = renderer.take_rejection();
      if (rejection) {
        recordPointerEventObservation("H3", "command_rejected", event);
        status.textContent = rejection;
        stopped = /revoked|expired/i.test(rejection);
      }
    } catch (error) {
      recordPointerEventObservation("H3", "server_message_failed", event);
      const description = error instanceof Error
        ? error.message
        : String(error);
      status.textContent = description;
      send(renderer.request_snapshot());
    }
  });
  connection.addEventListener("close", () => {
    recordPointerEventObservation("H3", "socket_closed", undefined);
    if (socket !== connection || stopped) return;
    status.textContent = "Disconnected — reconnecting…";
    setTimeout(() => connect(share), reconnectDelay);
    reconnectDelay = Math.min(reconnectDelay * 2, 30000);
  });
}

start().catch((error) => { status.textContent = error.message; });
