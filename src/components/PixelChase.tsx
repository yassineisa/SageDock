/** Small vector sprites stay crisp at Windows display scales without image downloads. */
export function PixelChase() {
  return (
    <div className="pixel-chase" role="img" aria-label="A pixel cat chasing the letter B">
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
            className="pixel-letter"
            viewBox="0 0 8 12"
            aria-hidden="true"
            shapeRendering="crispEdges"
          >
            <path
              d="M1 1H5V2H6V5H5V6H6V7H7V10H6V11H1ZM3 3V5H4V3ZM3 7V9H5V7Z"
              fill="currentColor"
              fillRule="evenodd"
            />
          </svg>
        </div>
      </div>
    </div>
  );
}
