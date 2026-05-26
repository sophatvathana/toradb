# ToraDB documentation (Mintlify)

Published documentation source for [ToraDB](https://github.com/sophatvathana/toradb). Content lives in `.mdx` files; navigation is defined in `docs.json`.

## Preview locally

```bash
npm i -g mint
cd mdx
mint dev
```

Open `http://localhost:3000`.

## Deploy

Connect this directory to [Mintlify](https://mintlify.com) via the GitHub app (set the docs root to `mdx/`). After deploy, update the site URL in the repository [`docs/README.md`](../docs/README.md).

## Branding

- `logo/toradb.png` — navbar logo (tiger icon)
- `favicon.png` — browser tab icon

## Structure

- `index.mdx` — home
- `concepts/` — mental model and on-disk layout
- `guides/` — tutorials
- `api-reference/` — Python, SQL, CLI
- `community/` — contributing, security, code of conduct
