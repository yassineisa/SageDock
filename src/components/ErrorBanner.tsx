import { useState } from "react";
import { Icon } from "./Icon";

/**
 * A friendly title and explanation, with technical detail behind an explicit toggle —
 * the product's progressive-disclosure rule in component form.
 *
 * Styling lives entirely in `layout.css`. An earlier version carried inline styles that
 * referenced design tokens directly, which silently broke the moment those tokens were
 * renamed; class names fail visibly instead.
 */
export function ErrorBanner({
  title,
  message,
  technicalDetails,
  tone = "error",
}: {
  title: string;
  message: string;
  technicalDetails?: string | null;
  tone?: "warning" | "error";
}) {
  const [showDetails, setShowDetails] = useState(false);

  return (
    <div className={tone === "warning" ? "banner banner--warning" : "banner"} role="alert">
      <Icon name={tone === "warning" ? "warning" : "error"} size={16} />
      <div className="banner__body">
        <p className="banner__title">{title}</p>
        <p className="banner__message">{message}</p>

        {technicalDetails ? (
          <>
            <div className="btn-row" style={{ marginTop: "var(--sp-3)" }}>
              <button
                type="button"
                className="btn"
                onClick={() => setShowDetails((v) => !v)}
                aria-expanded={showDetails}
              >
                {showDetails ? "Hide details" : "Show details"}
              </button>
            </div>
            {showDetails ? <pre>{technicalDetails}</pre> : null}
          </>
        ) : null}
      </div>
    </div>
  );
}
