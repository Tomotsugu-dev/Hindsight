# Internal GitHub Insights

Private repository analytics at
`https://hindsight.kyosweb.com/internal/github-insights/`.

This is a separate `hindsight-insights` Cloudflare Worker. Its two path routes
take precedence over the public website's Custom Domain, so its release cycle
does not rebuild the landing pages or desktop application.

## Access boundary

- Configure a Cloudflare Access application for this Worker or its internal
  path, with an explicit allowed identity. Initially only the site owner is
  allowed. Configure the application's audience tag as `ACCESS_AUD` in
  `wrangler.jsonc`.
- The handler requires platform-verified `ctx.access` with the expected audience
  on HTML, CSS, JavaScript and every API request. Missing configuration or Access
  context denies requests. Forged identity/JWT headers are not accepted.
- Do not add an `assets` binding: Cloudflare's internal Static Assets router
  does not currently forward `ctx.access` to the user Worker. The small dashboard
  files and shared download module are bundled as text and served after
  authentication instead.
- Both `workers_dev` and `preview_urls` are disabled. The handler also checks
  the canonical hostname. All responses are private/no-store and noindex.
- Traffic records live only in private KV. No report JSON is included in the
  Worker bundle, public Git repository, static website, or CI artifacts.

The Access policy is managed in Cloudflare. Treat a policy change as an access
change; keep it restricted to the intended internal audience. Creating this
Worker alone does not create an Access policy: the application and audience
must be configured separately before authorized visitors can enter.

## Collection and deployment

See [the collector documentation](../scripts/insights/README.md) for repository
selection, token permissions, daily schedule and data definitions.

`.github/workflows/insights-web.yml` verifies and packages pull requests, then
deploys this Worker on relevant pushes to `main`. It reuses the existing
`CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` Actions secrets.

```sh
node --test scripts/insights/*.test.mjs web-insights/worker.test.mjs
node --check web-insights/dashboard.js
npx wrangler@4.152.0 deploy --config web-insights/wrangler.jsonc --dry-run
npx wrangler@4.152.0 deploy --config web-insights/wrangler.jsonc
```

`scripts/insights/config.json` and this Worker's `INSIGHTS` binding must refer to
the same KV namespace. Namespace IDs and Access audience tags are identifiers,
not credentials. Never put API tokens in either configuration file.

## Dashboard

- Daily views, unique visitors, clones and unique cloners, with gaps preserved.
- Stars, forks, actual subscribers, and primary installer downloads.
- Installer totals by platform, compact version comparisons, and independent
  7/30-day download totals with daily bars. Signature files, update manifests, archives,
  and other release attachments do not count as installers.
- Repository switcher; 7/30/90-day or full-history charts; CSV export.
- Rolling 14-day Top 10 referrers and paths, explicitly distinct from daily data.
- Collection freshness, failure and first-run states. UTC dates remain UTC;
  daily unique counts are never presented as a cross-day deduplicated total.
- Daily detail is collapsed by default; the full attachment catalog is not
  rendered. Download increases require consecutive UTC snapshots and cannot
  reconstruct the dates of downloads before collection began.
- Period totals sum only known daily increases. Partial windows display the
  number of recorded days and label their total as recorded downloads; windows
  with no known increases show an unknown total instead of zero.

Public GitHub Actions scheduling is best effort. The website displays the latest
saved collection, not a live request to GitHub. Refreshing the page does not run
the collector. Daily job failures appear in Actions and in the saved status when
KV remains available. Monitor token expiry and the last-success timestamp.
