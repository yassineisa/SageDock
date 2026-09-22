/** Small vector sprites stay crisp at Windows display scales without image downloads. */
export function PixelChase() {
  return (
    <div className="pixel-chase" role="img" aria-label="A pixel cat chasing a bee">
      <div className="pixel-chase-runner">
        <div className="pixel-chase-direction">
          <svg
            className="pixel-cat"
            viewBox="0 0 20 16"
            aria-hidden="true"
            shapeRendering="crispEdges"
          >
            <g fill="currentColor">
              <path d="M2 5H4V9H7V7H12V3H14V5H17V3H19V11H17V13H6V11H2Z" />
              <g className="pixel-paws">
                <path d="M7 12H9V15H6V14H7ZM14 12H16V15H13V14H14Z" />
              </g>
            </g>
            <path d="M14 7H16V9H14Z" fill="var(--layer)" />
            <path d="M18 9H20V10H18Z" fill="var(--text)" />
          </svg>
          <svg
            className="pixel-bee"
            viewBox="0 0 12 16"
            aria-hidden="true"
            shapeRendering="crispEdges"
          >
            <path d="M3 3H5V4H6V7H3V6H2V4H3ZM7 2H9V3H10V5H9V7H6V4H7Z" fill="#8ECDE0" />
            <path d="M3 7H10V8H11V12H10V13H3V12H2V11H0V10H2V8H3Z" fill="#352B27" />
            <path d="M3 8H5V12H3ZM7 8H9V12H7ZM9 9H10V12H9Z" fill="#F4C34E" />
            <path d="M10 8H11V9H10Z" fill="#FFF7EE" />
          </svg>
        </div>
      </div>
    </div>
  );
}
