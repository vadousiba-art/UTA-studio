import { Notice } from "../components/ui";

// Replaced by the full Plugin Manager once the VST3 host lands (milestone 5).
export function Plugins() {
  return (
    <div className="workspace">
      <div className="page-header"><div><h1>Plugin Manager</h1></div></div>
      <Notice kind="info">Third-party VST3 hosting is under construction in this build.</Notice>
    </div>
  );
}
