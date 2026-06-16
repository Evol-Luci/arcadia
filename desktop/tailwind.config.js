/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        // Theme tokens are driven at runtime by CSS variables (Theme Engine),
        // exposed to Tailwind so utilities like `text-primary` follow the theme.
        primary: "rgb(var(--arc-primary) / <alpha-value>)",
        secondary: "rgb(var(--arc-secondary) / <alpha-value>)",
        surface: "rgb(var(--arc-surface) / <alpha-value>)",
        "surface-2": "rgb(var(--arc-surface-2) / <alpha-value>)",
        ink: "rgb(var(--arc-ink) / <alpha-value>)",
        "ink-dim": "rgb(var(--arc-ink-dim) / <alpha-value>)",
      },
      fontFamily: {
        display: ["Orbitron", "system-ui", "sans-serif"],
        sans: ["Inter", "system-ui", "sans-serif"],
      },
      boxShadow: {
        glow: "0 0 24px rgb(var(--arc-primary) / 0.45)",
        "glow-lg": "0 0 48px rgb(var(--arc-primary) / 0.55)",
      },
      keyframes: {
        "fade-up": {
          "0%": { opacity: "0", transform: "translateY(8px)" },
          "100%": { opacity: "1", transform: "translateY(0)" },
        },
        "progress-indeterminate": {
          "0%": { transform: "translateX(-100%)" },
          "100%": { transform: "translateX(400%)" },
        },
      },
      animation: {
        "fade-up": "fade-up 0.25s ease-out both",
        "progress-indeterminate": "progress-indeterminate 1.1s ease-in-out infinite",
      },
    },
  },
  plugins: [],
};
