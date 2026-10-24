import { Component, type ReactNode } from "react";

/** Shows unexpected UI errors instead of a blank window, with a reload button. */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: { componentStack?: string | null }) {
    console.error("UI error", error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="workspace">
        <div className="notice notice--danger" role="alert">
          <div className="grow">
            <strong>Something went wrong in the interface.</strong>
            <div style={{ marginTop: 6 }}>{this.state.error.message}</div>
            <div className="faint" style={{ marginTop: 6, fontSize: 11 }}>
              Your projects and recordings are saved on disk and are not affected. Reloading the window usually fixes this.
            </div>
            <button className="btn" style={{ marginTop: 10 }} onClick={() => window.location.reload()}>
              Reload window
            </button>
          </div>
        </div>
      </div>
    );
  }
}
