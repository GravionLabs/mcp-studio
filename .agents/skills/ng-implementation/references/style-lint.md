# Style lint: tokens only

Angular repositories enforce the tokens of `docs/STYLEGUIDE.md` (skill `ui-styleguide`) with Stylelint. Not ast-grep: Stylelint understands CSS and SCSS.

## Install

`setup-oxc-husky` installs `stylelint` and `stylelint-config-standard-scss`, copies its `templates/stylelint.config.mjs` to the repository root (the single source of the config, with the token rules) and adds `lint:styles` and the `lint-staged` entry (ADR 0008). By hand:

```powershell
pnpm add -D stylelint stylelint-config-standard-scss
```

Copy `stylelint.config.mjs` from the `setup-oxc-husky` templates. It uses `postcss-scss` (a dependency of `stylelint-config-standard-scss`), so `.css` and `.scss` files are read.

## Rules

- `color-no-hex`: no `#fff`.
- `function-disallowed-list`: no `rgb()`, `rgba()`, `hsl()`, `hsla()`; colors only as `var(--…)`.
- `declaration-property-value-allowed-list`: `font-size` is a token (`var(--…)`) or a step of the scale; adapt the `rem` list in the template to the scale of the guide.

Every message names the reason and `docs/STYLEGUIDE.md`.

## Gate

Add a fast check to `.agents/config.json`:

```json
{
  "checks": [
    {
      "name": "stylelint",
      "command": "npx --no-install stylelint \"src/**/*.{css,scss}\"",
      "fast": true
    }
  ]
}
```

## Token file

The token definitions may use raw values. The `overrides` entry of the config switches the three rules off for `tokens.css`, `tokens.scss`, `_tokens.scss` and `styles/tokens/`; change its `files` list to where the repository defines its tokens. Allow nothing else: a one-off value elsewhere is a new token in the guide.
