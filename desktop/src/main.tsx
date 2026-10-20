import { Component, StrictMode, type ErrorInfo, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { logFrontend } from "./api";
import "./styles.css";

/** A render error would otherwise leave a blank window with no clue. */
class Boundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    logFrontend("error", `render: ${error.stack ?? error.message}\n${info.componentStack ?? ""}`);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="page boot-state">
        <h1>Grok Desktop hit an error / 出现错误</h1>
        <pre className="boot-error">{this.state.error.message}</pre>
        <div className="row">
          <button className="primary" onClick={() => this.setState({ error: null })}>Retry / 重试</button>
          <button className="ghost" onClick={() => window.location.reload()}>Reload / 重新加载</button>
        </div>
      </div>
    );
  }
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Boundary>
      <App />
    </Boundary>
  </StrictMode>,
);
