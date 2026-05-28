"use strict";

const ByteArray = imports.byteArray;
const GLib = imports.gi.GLib;
const GObject = imports.gi.GObject;
const Main = imports.ui.main;
const PanelMenu = imports.ui.panelMenu;
const Shell = imports.gi.Shell;
const Soup = imports.gi.Soup;
const St = imports.gi.St;

const BRIDGE_URL = "http://127.0.0.1:45173/v1/gnome/window";
const EXTENSION_UUID = "devflow-recorder@local";
const HEARTBEAT_INTERVAL_MS = 5000;
const DIAG_PATH = GLib.build_filenamev([
  GLib.get_user_cache_dir(),
  "devflow-recorder",
  "gnome-extension.log"
]);

let indicator = null;
let session = null;
let timerId = 0;
let lastWindowKey = "";
let lastSentAt = 0;
let lastError = "";

function init() {
}

function enable() {
  debug("enable");
  session = new Soup.Session();
  try {
    indicator = new DevFlowIndicator();
    Main.panel.addToStatusArea(EXTENSION_UUID, indicator);
  } catch (error) {
    indicator = null;
    debug(`indicator failed: ${error.message || String(error)}`);
  }

  sendFocusedWindow();
  timerId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 1, () => {
    sendFocusedWindow();
    return GLib.SOURCE_CONTINUE;
  });
}

function disable() {
  debug("disable");
  if (timerId) {
    GLib.source_remove(timerId);
    timerId = 0;
  }

  if (session) {
    session.abort();
    session = null;
  }

  if (indicator) {
    indicator.destroy();
    indicator = null;
  }

  lastWindowKey = "";
  lastSentAt = 0;
}

var DevFlowIndicator = GObject.registerClass(
class DevFlowIndicator extends PanelMenu.Button {
  _init() {
    super._init(0.0, "DevFlow Recorder");
    this._label = new St.Label({
      text: "DF",
      style_class: "devflow-recorder-label"
    });
    this.add_child(this._label);
  }

  setOnline(isOnline) {
    if (!this._label) {
      return;
    }

    this._label.set_text(isOnline ? "DF" : "DF!");
  }
});

function sendFocusedWindow() {
  const payload = readFocusedWindow();
  if (!payload || !session) {
    diag(payload ? "skip:no-session" : "skip:no-focus");
    return;
  }

  const windowKey = [
    payload.title,
    payload.appId || "",
    payload.wmClass || "",
    payload.pid || ""
  ].join("|");

  const now = Date.now();
  if (windowKey === lastWindowKey && now - lastSentAt < HEARTBEAT_INTERVAL_MS) {
    diag("skip:heartbeat");
    return;
  }

  const body = JSON.stringify(payload);
  const message = Soup.Message.new("POST", BRIDGE_URL);
  message.request_headers.append("Content-Type", "application/json");
  const bridgeToken = readBridgeToken();
  if (bridgeToken) {
    message.request_headers.append("X-DevFlow-Token", bridgeToken);
  }

  if (message.set_request && session.queue_message) {
    diag("send:soup2");
    sendWithSoup2(message, body, windowKey, now);
  } else if (message.set_request_body_from_bytes && session.send_and_read_async) {
    diag("send:soup3");
    sendWithSoup3(message, body, windowKey, now);
  } else {
    debugOnce("unsupported libsoup API");
  }
}

function sendWithSoup2(message, body, windowKey, sentAt) {
  message.set_request("application/json", Soup.MemoryUse.COPY, body);
  session.queue_message(message, (_session, response) => {
    handleBridgeResponse(response.status_code, null, windowKey, sentAt);
  });
}

function sendWithSoup3(message, body, windowKey, sentAt) {
  message.set_request_body_from_bytes("application/json", GLib.Bytes.new(body));
  session.send_and_read_async(message, GLib.PRIORITY_DEFAULT, null, (_session, result) => {
    try {
      session.send_and_read_finish(result);
      const status = message.get_status ? message.get_status() : message.status_code;
      handleBridgeResponse(status, null, windowKey, sentAt);
    } catch (error) {
      handleBridgeResponse(0, error, windowKey, sentAt);
    }
  });
}

function handleBridgeResponse(status, error, windowKey, sentAt) {
  const ok = status >= 200 && status < 300;
  diag(ok ? `ok:${status}` : `fail:${status}`);
  if (ok) {
    lastWindowKey = windowKey;
    lastSentAt = sentAt;
    lastError = "";
  } else if (error) {
    debugOnce(error.message || String(error));
  } else {
    debugOnce(`bridge returned HTTP ${status}`);
  }

  if (indicator) {
    indicator.setOnline(ok);
  }
}

function readFocusedWindow() {
  const window = global.display.get_focus_window
    ? global.display.get_focus_window()
    : global.display.focus_window;
  if (!window) {
    return null;
  }

  const title = safeString(window.get_title ? window.get_title() : "");
  if (!title) {
    return null;
  }

  const tracker = Shell.WindowTracker.get_default();
  const app = tracker.get_window_app(window);
  const workspace = window.get_workspace ? window.get_workspace() : null;

  return {
    title,
    app: app ? safeString(app.get_name()) : null,
    appId: app ? safeString(app.get_id()) : null,
    wmClass: window.get_wm_class ? safeString(window.get_wm_class()) : null,
    pid: window.get_pid ? window.get_pid() : null,
    workspace: workspace ? workspace.index() : null,
    focusedAt: Date.now()
  };
}

function safeString(value) {
  if (value === null || value === undefined) {
    return "";
  }

  return String(value).slice(0, 160);
}

function readBridgeToken() {
  try {
    const path = GLib.build_filenamev([
      GLib.get_user_data_dir(),
      "devflow-recorder",
      "bridge-token"
    ]);
    const [ok, contents] = GLib.file_get_contents(path);
    if (!ok) {
      return "";
    }

    return ByteArray.toString(contents).trim();
  } catch (_error) {
    return "";
  }
}

function debugOnce(message) {
  if (!message || message === lastError) {
    return;
  }

  lastError = message;
  debug(`error:${message}`);
}

function debug(message) {
  log(`DevFlow Recorder Bridge: ${message}`);
  diag(message);
}

function diag(message) {
  try {
    const dir = GLib.path_get_dirname(DIAG_PATH);
    GLib.mkdir_with_parents(dir, 0o700);
    const line = `${new Date().toISOString()} ${message}\n`;
    GLib.file_set_contents(DIAG_PATH, readDiagTail() + line);
  } catch (_error) {
  }
}

function readDiagTail() {
  try {
    const [ok, contents] = GLib.file_get_contents(DIAG_PATH);
    if (!ok) {
      return "";
    }

    return ByteArray.toString(contents).split("\n").slice(-80).join("\n");
  } catch (_error) {
    return "";
  }
}
