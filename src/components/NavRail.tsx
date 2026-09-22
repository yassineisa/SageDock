import { NavLink } from "react-router-dom";
import { useRef, useState } from "react";
import logo from "../../src-tauri/icons/icon.svg";
import { Icon, type IconName } from "./Icon";
import { PixelChase } from "./PixelChase";

/**
 * The compact navigation pane, following the Windows NavigationView pattern: icon plus
 * label, a subtle fill for the selected item, and a short accent bar on its leading edge.
 *
 * Each link carries an explicit `aria-label`. Below 900px the pane collapses to icons only,
 * and without the label the accessible name would disappear with it, leaving an icon-only
 * navigation that no screen reader could describe at the app's minimum window size.
 */
const LINKS: [path: string, label: string, icon: IconName][] = [
  ["/", "Workspace", "home"],
  ["/tools", "Scientific tools", "calculator"],
  ["/recovery", "Recovery", "repair"],
  ["/diagnostics", "Diagnostics", "diagnostic"],
  ["/settings", "Settings", "settings"],
  ["/help", "Guides & help", "help"],
  ["/about", "About", "info"],
];

export function NavRail() {
  const clicks = useRef(0);
  const [chasing, setChasing] = useState(false);
  const clickLogo = () => {
    if (chasing) {
      setChasing(false);
      clicks.current = 0;
    } else if (++clicks.current === 10) {
      setChasing(true);
      clicks.current = 0;
    }
  };
  return (
    <aside className="nav-pane">
      <div className="brand">
        <button
          type="button"
          className="brand-logo"
          aria-label={chasing ? "Hide pixel chase" : "SageDock logo"}
          title={chasing ? "Hide pixel chase" : "SageDock"}
          onClick={clickLogo}
        >
          <img src={logo} alt="" draggable={false} />
        </button>
        <NavLink to="/" className="brand-text" aria-label="SageDock home">
          <span className="brand-name">SageDock</span>
          <span className="brand-sub">Scientific workspace</span>
        </NavLink>
      </div>

      <nav className="nav" aria-label="Main">
        {LINKS.map(([path, label, icon]) => (
          <NavLink key={path} to={path} end={path === "/"} aria-label={label}>
            <Icon name={icon} size={16} />
            <span>{label}</span>
          </NavLink>
        ))}
      </nav>

      <div className="nav-bottom">
        {chasing && <PixelChase />}
        <div className="nav-foot">
          <span className="small" style={{ fontWeight: 600 }}>
            Files stay in Windows
          </span>
          <p>Notebooks are saved in your Documents folder, outside the computing environment.</p>
        </div>
      </div>
    </aside>
  );
}
