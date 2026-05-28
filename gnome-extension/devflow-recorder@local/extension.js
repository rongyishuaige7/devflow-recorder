"use strict";

imports.gi.versions.Soup = "3.0";

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

let indicator = null;
let session = null;
let timerId = 0;
let lastWindowKey = "";
let lastSentAt = 0;
let lastError = "";

function init() {
}

function enable() {
  session = new Soup.Session();
  indicator = new DevFlowIndicator();
  Main.panel.addToStatusArea(EXTENSION_UUID, indicator);

  sendFocusedWindow();
  timerId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 1, () => {
    sendFocusedWindow();
    return GLib.SOURCE_CONTINUE;
  });
}

function disable() {
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
    return;
  }

  const body = JSON.stringify(payload);
  const message = Soup.Message.new("POST", BRIDGE_URL);
  message.request_headers.append("Content-Type", "application/json");
  const bridgeToken = readBridgeToken();
  if (bridgeToken) {
    message.request_headers.append("X-DevFlow-Token", bridgeToken);
  }
  message.set_request_body_from_bytes("application/json", GLib.Bytes.new(body));

  session.send_and_read_async(message, GLib.PRIORITY_DEFAULT, null, (_session, result) => {
    let ok = false;
    try {
      session.send_and_read_finish(result);
      const status = message.get_status ? message.get_status() : message.status_code;
      ok = status >= 200 && status < 300;
      if (!ok) {
        debugOnce(`bridge returned HTTP ${status}`);
      }
    } catch (error) {
      debugOnce(error.message || String(error));
    }

    if (ok) {
      lastWindowKey = windowKey;
      lastSentAt = now;
      lastError = "";
    }

    if (indicator) {
      indicator.setOnline(ok);
    }
  });
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
  log(`DevFlow Recorder Bridge: ${message}`);
}
