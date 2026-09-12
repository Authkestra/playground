# UI conventions

The playground's frontend is shadcn/ui on Tailwind 3, dark by construction.
This file is the contract every component in `apps/web/components` follows. It
exists because the UI it replaced had no contract: colour lived in forty
`className` strings, the one switch in the app was hand-measured with a code
comment explaining its 2px asymmetry, and `focus-visible` was styled per
component, so some panels had no focus ring at all.

## Colour: tokens only

Never write a palette class. No `bg-slate-900`, no `text-emerald-400`, no
`border-slate-700`. Address colour only through the semantic tokens declared in
`app/globals.css`:

| Token | Use for |
|---|---|
| `background` / `foreground` | The page and its body text |
| `card` / `card-foreground` | Any raised panel: a scenario card, a ceremony panel |
| `popover` / `popover-foreground` | Anything floating above a card |
| `muted` / `muted-foreground` | Inert surfaces; captions, helper text, secondary labels |
| `primary` / `primary-foreground` | The active state of a control, the forward action of a step, the focus ring |
| `secondary` / `accent` | Hover and pressed surfaces, segmented-control selection |
| `success` / `success-foreground` | A ceremony that completed, a token accepted |
| `warning` / `warning-foreground` | A pending or degraded state |
| `destructive` / `destructive-foreground` | A rejection, a forged signature, a hard error |
| `border` / `input` | Hairlines; the resting track of a control |

`DEFAULT` is the surface, `-foreground` the text that sits on it. For coloured
*text on the page* (a verdict line, not a filled badge), use the `-foreground`
shade — those are tuned to clear WCAG AA on both `background` and `card`, which
the raw 400-weight palette shades did not.

Opacity modifiers compose (`bg-primary/10`, `border-destructive/40`) because
the tokens are bare `H S% L%` triples. That is the sanctioned way to get a tint.

## Why this is enforced, not just asked

`scripts/contrast-audit.mjs` resolves these tokens out of `globals.css` and
checks every `text-*` against the surface it actually sits on. It also asserts a
floor on the number of pairs it checked: an audit that silently stops
recognising the classes in use is worse than no audit, because it reports green.
A palette class slipped into a component is therefore either caught as a
contrast failure or caught as an unresolvable token — not ignored.

## Components

Reach for `@/components/ui/*` before writing markup:

- `Switch` for a boolean scenario toggle — never a hand-rolled `role="switch"`.
- `Checkbox` + `Label` for `select_many`, `RadioGroup` for `select_one`. Both
  replace bare native inputs, which previously carried no classes at all and so
  rendered at OS default size next to styled controls.
- `Button` for every action. `variant="default"` is the step's forward action
  and there is at most one per view; `secondary` for Back; `outline` for a
  side action; `ghost` for anything in a toolbar; `destructive` only for a
  genuinely destructive act. `size="sm"` inside panels, default at step level.
- `Card` for every panel, with `CardHeader`/`CardTitle`/`CardDescription`/
  `CardContent`/`CardFooter` rather than divs with padding.
- `Badge` for a status pill. `Alert` for a banner. `Separator` for a rule.
- `Tabs` for the OAuth session/JWT mode picker.
- `Tooltip` for the "why is this disabled" affordance.

Import `cn` from `@/lib/cn` and use it for every conditional class. Template
literals do not merge conflicting utilities, so a caller's `className` cannot
override a component default.

## Focus and motion

Do not style `focus-visible` per component. `globals.css` gives every
interactive element the same ring. Auth flows are keyboard-heavy and a
per-component ring is how the old UI ended up with panels that had none.

Transitions are `transition-colors` on hover/active states. Anything that moves
a box does so under `duration-200 ease-out` or not at all.

## Accessibility

Keep the semantics the old components got right: `aria-checked` on switches
comes from Radix now, but `aria-label`, `aria-describedby`, `aria-live` on
verdict regions, and the disabled-reason text are still the component's job. A
ceremony result must be announced, not merely recoloured — colour is the second
signal, never the only one.

## What must not change

Each component file exports small pure helpers that are unit-tested in a
sibling `.test.ts` (`isControlValueActive`, `visiblePanels`, `verdictStyle`,
`normalizeCode`, `PANEL_ORDER`, the base64url helpers, and friends). The tests
run in a `node` environment and never render JSX. Rewriting markup is free;
renaming or changing the signature of one of those exports breaks the suite.
