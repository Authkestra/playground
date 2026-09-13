import type { Config } from "tailwindcss";

/*
  Colours are declared here only as names pointing at the CSS variables in
  `app/tokens.css`. The `<alpha-value>` placeholder is what keeps the opacity
  modifier working — `bg-card/60` resolves because Tailwind substitutes the
  alpha into the `hsl()` call rather than the variable carrying its own.
*/
const token = (name: string) => `hsl(var(--${name}) / <alpha-value>)`;

/*
  Everything below this point (fonts, the type ramp, shadows, motion) reaches
  back into the same `--ak-*` primitives, kept as CSS variables rather than
  copied into this file as literals so that a change to `tokens.css` — the
  copy of the canonical file — takes effect here without a second edit.
  `--ak-space-*` is deliberately not wired up: it is a plain 4px ramp, which
  is exactly Tailwind's own default spacing scale, so there is nothing to add.
*/

const config: Config = {
  content: [
    "./app/**/*.{ts,tsx}",
    "./components/**/*.{ts,tsx}",
    "./lib/**/*.{ts,tsx}",
  ],
  theme: {
    container: {
      center: true,
      padding: "1.5rem",
      screens: { "2xl": "1200px" },
    },
    extend: {
      colors: {
        border: token("border"),
        input: token("input"),
        ring: token("ring"),
        background: token("background"),
        foreground: token("foreground"),
        primary: {
          DEFAULT: token("primary"),
          foreground: token("primary-foreground"),
          accent: token("primary-accent"),
          // Tinted ground behind accent *text*, at panel scale at most — see
          // DESIGN.md §2. It is not a general-purpose surface colour, and
          // reaching for it as one is exactly the "warm-tinted surface"
          // mistake this rework exists to undo.
          subtle: token("primary-subtle"),
        },
        // The mark itself — the logo and the favicon. Never UI chrome, so it
        // has no `-foreground` pair and nothing in `components/` should
        // reach for it.
        brand: token("brand"),
        secondary: {
          DEFAULT: token("secondary"),
          foreground: token("secondary-foreground"),
        },
        destructive: {
          DEFAULT: token("destructive"),
          foreground: token("destructive-foreground"),
          muted: token("destructive-muted"),
        },
        success: {
          DEFAULT: token("success"),
          foreground: token("success-foreground"),
        },
        warning: {
          DEFAULT: token("warning"),
          foreground: token("warning-foreground"),
        },
        info: {
          DEFAULT: token("info"),
          foreground: token("info-foreground"),
        },
        muted: {
          DEFAULT: token("muted"),
          foreground: token("muted-foreground"),
        },
        accent: {
          DEFAULT: token("accent"),
          foreground: token("accent-foreground"),
        },
        popover: {
          DEFAULT: token("popover"),
          foreground: token("popover-foreground"),
        },
        card: {
          DEFAULT: token("card"),
          foreground: token("card-foreground"),
        },
      },
      fontFamily: {
        // Inter for everything a person reads, JetBrains Mono for everything
        // a compiler reads — see DESIGN.md §3. The variables already carry
        // their own fallback stacks, so each is a single entry here.
        sans: ["var(--ak-font-sans)"],
        mono: ["var(--ak-font-mono)"],
      },
      /*
        The type ramp from DESIGN.md §3, reproduced key for key: each step
        pairs its size with the leading and tracking the table specifies,
        rather than leaving line-height and letter-spacing to the values
        Tailwind's own scale would otherwise supply. Tracking tightens as
        size grows — Inter at 48px with default tracking looks loose next to
        the same face at 16px — and the `2xs` step is new (Tailwind has no
        equivalent) for all-caps eyebrows.
      */
      fontSize: {
        "2xs": [
          "var(--ak-text-2xs)",
          { lineHeight: "var(--ak-leading-normal)", letterSpacing: "var(--ak-tracking-wide)" },
        ],
        xs: [
          "var(--ak-text-xs)",
          { lineHeight: "var(--ak-leading-normal)", letterSpacing: "var(--ak-tracking-snug)" },
        ],
        sm: [
          "var(--ak-text-sm)",
          { lineHeight: "var(--ak-leading-normal)", letterSpacing: "var(--ak-tracking-snug)" },
        ],
        base: [
          "var(--ak-text-base)",
          { lineHeight: "var(--ak-leading-relaxed)", letterSpacing: "var(--ak-tracking-snug)" },
        ],
        lg: [
          "var(--ak-text-lg)",
          { lineHeight: "var(--ak-leading-relaxed)", letterSpacing: "var(--ak-tracking-snug)" },
        ],
        xl: [
          "var(--ak-text-xl)",
          { lineHeight: "var(--ak-leading-snug)", letterSpacing: "var(--ak-tracking-snug)" },
        ],
        "2xl": [
          "var(--ak-text-2xl)",
          { lineHeight: "var(--ak-leading-snug)", letterSpacing: "var(--ak-tracking-tight)" },
        ],
        "3xl": [
          "var(--ak-text-3xl)",
          { lineHeight: "var(--ak-leading-snug)", letterSpacing: "var(--ak-tracking-tight)" },
        ],
        "4xl": [
          "var(--ak-text-4xl)",
          { lineHeight: "var(--ak-leading-tight)", letterSpacing: "var(--ak-tracking-tight)" },
        ],
        "5xl": [
          "var(--ak-text-5xl)",
          { lineHeight: "var(--ak-leading-tight)", letterSpacing: "var(--ak-tracking-tight)" },
        ],
      },
      borderRadius: {
        lg: "var(--radius)",
        md: "calc(var(--radius) - 2px)",
        sm: "calc(var(--radius) - 4px)",
        // Code blocks and large panels, per DESIGN.md §4 — one step past
        // `lg` rather than a value of its own, so rounding the whole product
        // is still the one-line change `--radius` promises.
        xl: "var(--ak-radius-xl)",
      },
      boxShadow: {
        "ak-1": "var(--ak-shadow-1)",
        "ak-2": "var(--ak-shadow-2)",
        "ak-3": "var(--ak-shadow-3)",
        // The one place a glow is allowed: the hero's primary call to
        // action. Not a general-purpose "make it orange" shadow.
        "ak-accent": "var(--ak-shadow-accent)",
      },
      transitionDuration: {
        "ak-fast": "var(--ak-duration-fast)",
        "ak-base": "var(--ak-duration-base)",
        "ak-slow": "var(--ak-duration-slow)",
      },
      transitionTimingFunction: {
        "ak-out": "var(--ak-ease-out)",
        "ak-in-out": "var(--ak-ease-in-out)",
      },
      keyframes: {
        "accordion-down": {
          from: { height: "0" },
          to: { height: "var(--radix-accordion-content-height)" },
        },
        "accordion-up": {
          from: { height: "var(--radix-accordion-content-height)" },
          to: { height: "0" },
        },
      },
      animation: {
        "accordion-down": "accordion-down 0.2s ease-out",
        "accordion-up": "accordion-up 0.2s ease-out",
      },
    },
  },
  plugins: [require("tailwindcss-animate")],
};

export default config;
