import Link from "next/link";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "How it works — Authkestra Playground",
  description:
    "What the playground actually does across its three steps, which parts run for real, and what the download contains.",
};

/**
 * A pure server component — no client boundary anywhere in its tree, which is
 * the point: this route should ship nothing beyond the framework's own
 * baseline runtime. See `scripts/bundle-budget.mjs`, which budgets it near
 * the `/_not-found` floor for exactly that reason.
 */
export default function HowItWorksPage() {
  return (
    <main className="mx-auto flex w-full max-w-3xl flex-1 flex-col gap-10 px-6 py-10 sm:py-14">
      <div className="flex flex-col gap-3">
        <h1 className="text-3xl font-semibold tracking-tight text-foreground">
          How this works
        </h1>
        <p className="text-base text-muted-foreground">
          What the playground does, which parts of it are real, and what you get
          out of it.
        </p>
      </div>

      <section className="flex flex-col gap-3">
        <h2 className="text-xl font-semibold tracking-tight text-foreground">
          What this is
        </h2>
        <p className="text-base leading-relaxed text-foreground">
          This site is the playground, not the framework. Authkestra itself —
          the crates, the engine, the adapters — lives in its own repository
          and has its own roadmap; this one only demonstrates it and generates
          starter projects from it. Confusing the two is easy to do by
          accident, since both are called authkestra, so it is worth being
          direct about it up front rather than leaving you to work it out from
          context.
        </p>
        <p className="text-base leading-relaxed text-foreground">
          Concretely: the playground lets you configure authentication for a
          hypothetical service — passkeys, a TOTP authenticator app, OAuth,
          bot protection — watch the generated Rust configuration change as
          you do, try the resulting flows in your browser, and then download a
          project that already has that configuration wired up.
        </p>
      </section>

      <section className="flex flex-col gap-4">
        <h2 className="text-xl font-semibold tracking-tight text-foreground">
          Three steps
        </h2>

        <div className="flex flex-col gap-2">
          <h3 className="text-base font-semibold text-foreground">
            1. Choose your methods
          </h3>
          <p className="text-base leading-relaxed text-muted-foreground">
            Each scenario — TOTP, passkeys, OAuth, bot protection — is a
            toggle or a small set of controls, and the controls are not a
            drawing of a configuration surface: every change is sent to the
            API, applied to your session, and answered with a diff of the Rust
            configuration that change actually produces. One definition per
            scenario drives the control, the diff and the download, which is
            why the three cannot disagree.
          </p>
        </div>

        <div className="flex flex-col gap-2">
          <h3 className="text-base font-semibold text-foreground">
            2. Sign in
          </h3>
          <p className="text-base leading-relaxed text-muted-foreground">
            Whichever methods you switched on in step one, you can then run for
            real: register a passkey with your device or enrol an authenticator
            app by scanning a QR code, then complete the corresponding sign-in
            ceremony. This is not a simulated success screen — it is the actual
            WebAuthn or TOTP flow talking to the actual API.
          </p>
        </div>

        <div className="flex flex-col gap-2">
          <h3 className="text-base font-semibold text-foreground">
            3. Download
          </h3>
          <p className="text-base leading-relaxed text-muted-foreground">
            The last step turns your configuration into a starter project:
            a zip you can unpack and build immediately, or a push straight to a
            repository of your own. Either way it carries the same scenarios
            you switched on, wired up rather than left as commented-out
            placeholders.
          </p>
        </div>
      </section>

      <section className="flex flex-col gap-3">
        <h2 className="text-xl font-semibold tracking-tight text-foreground">
          What is real, and what is not yet
        </h2>
        <p className="text-base leading-relaxed text-foreground">
          TOTP and passkeys run end to end against the real framework: signing
          in through this site mints real credentials, scoped to your demo
          session, with the same signature-counter tracking and code
          verification the framework ships. Nothing about those two flows is
          simulated.
        </p>
        <p className="text-base leading-relaxed text-foreground">
          OAuth and bot protection are built and tested to the same standard,
          but wait on provider credentials — a GitHub, Google or Discord OAuth
          app; a Turnstile, hCaptcha or reCAPTCHA site key. Without those, the
          control renders itself unavailable with a reason rather than
          pretending to work. Registering the credentials is configuration, not
          a code change.
        </p>
        <p className="text-base leading-relaxed text-foreground">
          There is also a protected route you can call with a token the
          playground has just issued. The route holds no secret: it fetches the
          issuer&rsquo;s published key set, matches on the key id in the
          token&rsquo;s header, and validates against that. You can fetch the
          same key set yourself and check the key id by hand — and you can have
          a token signed by a key that is deliberately absent from it, to watch
          the route refuse it for that reason by name rather than a flat error.
        </p>
        <p className="text-base leading-relaxed text-foreground">
          The configuration diff you see in step one and the project you
          download in step three are two outputs of one scenario definition,
          and a test asserts that the crates and feature flags the diff names
          are the ones the generated <code>Cargo.toml</code> actually carries.
          They are checked against each other rather than trusted to agree, so
          a diff that promises something the download does not deliver fails
          the build instead of reaching you.
        </p>
      </section>

      <section className="flex flex-col gap-3">
        <h2 className="text-xl font-semibold tracking-tight text-foreground">
          The download
        </h2>
        <p className="text-base leading-relaxed text-foreground">
          What step three produces is a real, compiling Cargo project — not a
          snippet to paste into one. The scenarios you chose are already
          assembled into its dependencies, its feature flags and its
          configuration, so the first thing you do with it is{" "}
          <code>cargo run</code> rather than an afternoon of reconciling
          imports and guessing which crate a feature lives on.
        </p>
      </section>

      <p className="text-base">
        <Link href="/" className="font-medium text-primary-accent hover:underline">
          Open the playground
        </Link>
      </p>
    </main>
  );
}
