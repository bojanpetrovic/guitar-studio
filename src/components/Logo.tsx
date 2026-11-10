export default function Logo({ size = 24 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 1024 1024" aria-hidden>
      <defs>
        <linearGradient id="logo-pick" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" style={{ stopColor: "var(--grad-a)" }} />
          <stop offset="1" style={{ stopColor: "var(--grad-c)" }} />
        </linearGradient>
      </defs>
      <path
        d="M512 132 C 740 132 872 220 872 362 C 872 556 676 762 512 882 C 348 762 152 556 152 362 C 152 220 284 132 512 132 Z"
        fill="url(#logo-pick)"
      />
      <path
        d="M282 440 L 372 440 L 422 322 L 492 594 L 564 256 L 632 530 L 672 440 L 752 440"
        fill="none"
        stroke="var(--on-accent)"
        strokeWidth="44"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
