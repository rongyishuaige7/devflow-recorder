import React, { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import {
  countUsableProviders,
  formatDuration,
  formatTotalDuration,
  totalDurationSeconds,
  type ProviderState
} from "./activity";
import "./styles.css";

type ProviderStatus = {
  id: string;
  name: string;
  state: ProviderState;
  detail: string;
};

type EnvironmentSnapshot = {
  sessionType: string;
  desktop: string;
  compositor: string;
  waylandDisplay?: string;
  x11Display?: string;
  providers: ProviderStatus[];
};

type ActiveWindow = {
  app: string;
  title: string;
  source: string;
  pid?: number;
};

type ActivityEvent = {
  id: string;
  startedAt: string;
  endedAt: string;
  durationSeconds?: number;
  durationMinutes: number;
  app: string;
  title: string;
  context: string;
  kind: string;
  confidence: number;
  privacy: string;
};

type RecorderSnapshot = {
  environment: EnvironmentSnapshot;
  activeWindow?: ActiveWindow;
  events: ActivityEvent[];
};

type SnapshotMode = "tauri" | "preview";

const stateLabel: Record<ProviderState, string> = {
  ready: "可用",
  partial: "部分",
  available: "兜底",
  planned: "计划",
  standby: "待机"
};

const stateTone: Record<ProviderState, string> = {
  ready: "good",
  partial: "warn",
  available: "muted",
  planned: "info",
  standby: "quiet"
};

const fallbackSnapshot: RecorderSnapshot = {
  environment: {
    sessionType: "browser-preview",
    desktop: "Tauri 后端未连接",
    compositor: "预览模式",
    providers: [
      {
        id: "hyprland",
        name: "Hyprland activewindow",
        state: "ready",
        detail: "Tauri 运行时会调用 hyprctl activewindow -j。"
      },
      {
        id: "sway",
        name: "Sway tree",
        state: "ready",
        detail: "Tauri 运行时会调用 swaymsg -t get_tree。"
      },
      {
        id: "kwin",
        name: "KDE KWin DBus",
        state: "partial",
        detail: "后续通过 KWin 脚本或 DBus 扩展接入。"
      },
      {
        id: "browser-extension",
        name: "浏览器扩展桥",
        state: "planned",
        detail: "扩展主动上报激活标签页标题与脱敏 URL。"
      }
    ]
  },
  activeWindow: {
    app: "Code",
    title: "devflow-recorder - Wayland collector",
    source: "browser preview"
  },
  events: [
    {
      id: "preview-1",
      startedAt: "09:20",
      endedAt: "10:05",
      durationMinutes: 45,
      app: "Code",
      title: "devflow-recorder - Rust collector",
      context: "项目：devflow-recorder / 分支：wayland-mvp",
      kind: "开发",
      confidence: 88,
      privacy: "仅记录窗口标题，不记录文件内容"
    },
    {
      id: "preview-2",
      startedAt: "10:06",
      endedAt: "10:22",
      durationMinutes: 16,
      app: "Firefox",
      title: "Wayland security model and desktop portals",
      context: "https://example.local/docs/wayland-security",
      kind: "资料",
      confidence: 74,
      privacy: "URL query 参数会被移除"
    },
    {
      id: "preview-3",
      startedAt: "10:23",
      endedAt: "10:36",
      durationMinutes: 13,
      app: "Terminal",
      title: "cargo test collector",
      context: "目录：/data/projects/devflow-recorder",
      kind: "验证",
      confidence: 81,
      privacy: "命令记录后续需要显式开启"
    }
  ]
};

// Inline SVG Icon components for premium feel
function TimelineIcon() {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="10" />
      <polyline points="12 6 12 12 16 14" />
    </svg>
  );
}

function CollectorsIcon() {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
      <line x1="8" y1="21" x2="16" y2="21" />
      <line x1="12" y1="17" x2="12" y2="21" />
    </svg>
  );
}

function ShieldIcon() {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
    </svg>
  );
}

function FileTextIcon() {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
      <polyline points="14 2 14 8 20 8" />
      <line x1="16" y1="13" x2="8" y2="13" />
      <line x1="16" y1="17" x2="8" y2="17" />
      <polyline points="10 9 9 9 8 9" />
    </svg>
  );
}

function CopyIcon() {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#10b981" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="20 6 9 17 4 12" />
    </svg>
  );
}

function getEventIcon(kind: string) {
  switch (kind) {
    case "开发":
      return (
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#60a5fa" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" style={{ width: 14, height: 14, marginRight: 6, verticalAlign: "middle" }}>
          <polyline points="16 18 22 12 16 6" />
          <polyline points="8 6 2 12 8 18" />
        </svg>
      );
    case "资料":
      return (
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#34d399" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" style={{ width: 14, height: 14, marginRight: 6, verticalAlign: "middle" }}>
          <path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z" />
          <path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z" />
        </svg>
      );
    case "验证":
      return (
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" style={{ width: 14, height: 14, marginRight: 6, verticalAlign: "middle" }}>
          <polyline points="4 17 10 11 4 5" />
          <line x1="12" y1="19" x2="20" y2="19" />
        </svg>
      );
    default:
      return (
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#9ca3af" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" style={{ width: 14, height: 14, marginRight: 6, verticalAlign: "middle" }}>
          <circle cx="12" cy="12" r="10" />
          <polyline points="12 6 12 12 16 14" />
        </svg>
      );
  }
}

function App() {
  const [snapshot, setSnapshot] = useState<RecorderSnapshot>(fallbackSnapshot);
  const [snapshotMode, setSnapshotMode] = useState<SnapshotMode>("preview");
  const [isRecording, setIsRecording] = useState(true);
  const [selectedFilter, setSelectedFilter] = useState("全部");
  const [error, setError] = useState<string | null>(null);
  const [activeHash, setActiveHash] = useState("#timeline");
  const [isCopied, setIsCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;

    async function loadSnapshot() {
      try {
        const next = await invoke<RecorderSnapshot>("get_recorder_snapshot");
        if (!cancelled) {
          setSnapshot(next);
          setSnapshotMode("tauri");
          setError(null);
        }
      } catch (err) {
        if (!cancelled) {
          setSnapshot(fallbackSnapshot);
          setSnapshotMode("preview");
          setError("当前是浏览器预览；启动 Tauri 桌面端后会读取真实 Wayland provider。");
        }
      }
    }

    loadSnapshot();
    const timer = window.setInterval(loadSnapshot, 7000);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  // Sync hash state with window url
  useEffect(() => {
    const handleHashChange = () => {
      if (window.location.hash) {
        setActiveHash(window.location.hash);
      }
    };
    window.addEventListener("hashchange", handleHashChange);
    handleHashChange();
    return () => window.removeEventListener("hashchange", handleHashChange);
  }, []);

  const filters = useMemo(() => {
    return ["全部", ...Array.from(new Set(snapshot.events.map((event) => event.kind)))];
  }, [snapshot.events]);
  const showFilters = filters.length > 2;

  const visibleEvents = useMemo(() => {
    if (selectedFilter === "全部") {
      return snapshot.events;
    }

    return snapshot.events.filter((event) => event.kind === selectedFilter);
  }, [selectedFilter, snapshot.events]);

  const totalSeconds = totalDurationSeconds(snapshot.events);
  const readyProviders = countUsableProviders(snapshot.environment.providers);
  const hasRealEvents = snapshotMode === "tauri" && snapshot.events.length > 0;
  const isTauriWithoutWindow = snapshotMode === "tauri" && !snapshot.activeWindow;

  const handleCopyDraft = () => {
    const draftText = hasRealEvents
      ? "当前已记录本地活动时间线。日报生成会基于 SQLite 中的活动与分段累计。"
      : "暂无真实活动数据。需要先接入你的桌面 provider 或浏览器扩展桥。";
    navigator.clipboard.writeText(draftText);
    setIsCopied(true);
    setTimeout(() => setIsCopied(false), 2000);
  };

  const handleRecordingToggle = async () => {
    const next = !isRecording;
    setIsRecording(next);
    try {
      await invoke<boolean>("set_recording_enabled", { enabled: next });
    } catch {
      setIsRecording(!next);
      setError("切换记录状态失败；Tauri 后端可能未连接。");
    }
  };

  return (
    <main className="app-shell">
      <section className="sidebar" aria-label="Recorder controls">
        <div className="brand-block">
          <div className="brand-mark">DF</div>
          <div>
            <p className="eyebrow">Wayland-first</p>
            <h1>DevFlow Recorder</h1>
          </div>
        </div>

        <button
          className={`record-switch ${isRecording ? "is-on" : ""}`}
          type="button"
          onClick={handleRecordingToggle}
          aria-pressed={isRecording}
        >
          <span className="switch-dot" />
          <span>{isRecording ? "记录中" : "已暂停"}</span>
        </button>

        <nav className="side-nav" aria-label="Views">
          <a 
            href="#timeline" 
            className={activeHash === "#timeline" ? "is-active" : ""}
            onClick={() => setActiveHash("#timeline")}
          >
            <TimelineIcon />
            <span>时间线</span>
          </a>
          <a 
            href="#providers" 
            className={activeHash === "#providers" ? "is-active" : ""}
            onClick={() => setActiveHash("#providers")}
          >
            <CollectorsIcon />
            <span>采集能力</span>
          </a>
          <a 
            href="#privacy" 
            className={activeHash === "#privacy" ? "is-active" : ""}
            onClick={() => setActiveHash("#privacy")}
          >
            <ShieldIcon />
            <span>隐私规则</span>
          </a>
          <a 
            href="#summary" 
            className={activeHash === "#summary" ? "is-active" : ""}
            onClick={() => setActiveHash("#summary")}
          >
            <FileTextIcon />
            <span>日报草稿</span>
          </a>
        </nav>

        <div className="env-card">
          <p>会话</p>
          <strong>{snapshot.environment.sessionType}</strong>
          <span>{snapshot.environment.compositor}</span>
        </div>
      </section>

      <section className="workspace">
        <header className="topbar">
          <div>
            <p className="eyebrow">本地工作流记忆</p>
            <h2>今天的开发轨迹</h2>
          </div>
          <div className="status-strip">
            <Metric label="累计" value={formatTotalDuration(totalSeconds)} />
            <Metric label="Provider" value={`${readyProviders}/${snapshot.environment.providers.length}`} />
            <Metric label="桌面" value={snapshot.environment.desktop} />
          </div>
        </header>

        {error ? <div className="error-banner">{error}</div> : null}
        {isTauriWithoutWindow ? (
          <div className="error-banner neutral">
            Tauri 后端已连接，但当前桌面没有暴露活跃窗口元数据。现在不会生成静态假数据；等接入你的桌面 provider 或浏览器扩展后，时间线才会出现真实记录。
          </div>
        ) : null}

        <section className="current-band">
          <div>
            <p className="eyebrow">当前焦点</p>
            <h3>{snapshot.activeWindow?.title ?? "尚未拿到活跃窗口"}</h3>
            <p>
              {snapshot.activeWindow
                ? `${snapshot.activeWindow.app} · ${snapshot.activeWindow.source}`
                : "Wayland 下需要 compositor 暴露窗口元数据；Hyprland/Sway 会优先尝试。"}
            </p>
          </div>
          <div className="privacy-pill">默认本地 · 不截图 · 不记录键盘</div>
        </section>

        <section className="content-grid">
          <div className="timeline-panel" id="timeline">
            <div className="panel-header">
              <div>
                <p className="eyebrow">Timeline</p>
                <h3>{snapshotMode === "preview" ? "活动时间线示例" : "活动时间线"}</h3>
              </div>
              {showFilters ? (
                <div className="segmented">
                  {filters.map((filter) => (
                    <button
                      key={filter}
                      type="button"
                      className={selectedFilter === filter ? "is-selected" : ""}
                      onClick={() => setSelectedFilter(filter)}
                    >
                      {filter}
                    </button>
                  ))}
                </div>
              ) : null}
            </div>

            {visibleEvents.length > 0 ? (
              <ol className="timeline-list">
                {visibleEvents.map((event) => (
                  <li key={event.id} className="timeline-item">
                    <time>
                      {event.startedAt}
                      <span>{event.endedAt}</span>
                    </time>
                    <div className="event-body">
                      <div className="event-title-row">
                        <strong>
                          {getEventIcon(event.kind)}
                          {event.title}
                        </strong>
                        <span>{formatDuration(event)}</span>
                      </div>
                      <p>{event.app} · {event.context}</p>
                      {snapshotMode === "preview" ? (
                        <div className="event-meta">
                          <span>{event.kind}</span>
                          <span>置信度 {event.confidence}%</span>
                          <span>{event.privacy}</span>
                          <span>示例数据</span>
                        </div>
                      ) : null}
                    </div>
                  </li>
                ))}
              </ol>
            ) : (
              <div className="empty-state">
                <strong>还没有真实活动事件</strong>
                <p>
                  暂无可展示的活动。GNOME 需要启用 Shell 扩展；Hyprland/Sway 会由后端后台采集。
                  SQLite 本地存储已启用，采集到活动后会自动恢复当天时间线。
                </p>
              </div>
            )}
          </div>

          <aside className="right-rail">
            <section className="rail-section" id="providers">
              <div className="panel-header compact">
                <div>
                  <p className="eyebrow">Collectors</p>
                  <h3>采集能力</h3>
                </div>
              </div>
              <div className="provider-list">
                {snapshot.environment.providers.map((provider) => (
                  <article key={provider.id} className="provider-card">
                    <div>
                      <strong>{provider.name}</strong>
                      <p>{provider.detail}</p>
                    </div>
                    <span className={`state-badge ${stateTone[provider.state]}`}>
                      {stateLabel[provider.state]}
                    </span>
                  </article>
                ))}
              </div>
            </section>

            <section className="rail-section" id="privacy">
              <p className="eyebrow">Privacy</p>
              <h3>第一版隐私边界</h3>
              <ul className="privacy-list">
                <li>窗口标题最长保留 160 字符</li>
                <li>URL 默认移除 query 与 fragment</li>
                <li>终端只读取进程名、argv0 basename 和 cwd，不读取命令参数</li>
                <li>活动与分段保存到本机 SQLite，不上传服务端</li>
              </ul>
            </section>

            <section className="rail-section summary" id="summary">
              <div className="panel-header compact">
                <div>
                  <p className="eyebrow">Draft</p>
                  <h3>中文日报草稿</h3>
                </div>
                <button
                  className="copy-btn"
                  onClick={handleCopyDraft}
                  title="复制草稿"
                  type="button"
                >
                  {isCopied ? <CheckIcon /> : <CopyIcon />}
                </button>
              </div>
              <div className="summary-box">
                <p style={{ margin: 0 }}>
                  {hasRealEvents
                    ? "当前已记录本地活动时间线。后续可以根据 SQLite 中的活动分段、应用和标题生成中文日报草稿。"
                    : "暂无真实活动数据。需要先启用桌面 provider 或浏览器扩展桥。"}
                </p>
              </div>
            </section>
          </aside>
        </section>
      </section>
    </main>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
