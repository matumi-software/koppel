# Repository rulesets

The branch and tag rules for this repository, kept here so they are reviewed like code. GitHub only
enforces rulesets on a private repository with a paid plan, so these are applied once the
repository is public.

| File | What it does |
|---|---|
| `branches.json` | `main` and `dev` cannot be deleted or force-pushed. No one bypasses it |
| `main-pull-requests.json` | Changes reach `main` through a pull request with resolved conversations. Admins may push directly, which is how releases are merged today |
| `release-tags.json` | `v*` tags cannot be deleted or moved once pushed |

Apply them, or apply a change to one, with the GitHub CLI:

```sh
gh api -X POST repos/matumi-software/koppel/rulesets --input .github/rulesets/branches.json
gh api repos/matumi-software/koppel/rulesets                                   # ids, for an update
gh api -X PUT repos/matumi-software/koppel/rulesets/<id> --input .github/rulesets/branches.json
```

The other settings that go with making the repository public:

```sh
# Secret scanning, push protection (blocks a push that contains a credential) and private
# vulnerability reporting, which SECURITY.md points reporters to
gh api -X PATCH repos/matumi-software/koppel --input - <<'JSON'
{ "security_and_analysis": {
    "secret_scanning": { "status": "enabled" },
    "secret_scanning_push_protection": { "status": "enabled" } } }
JSON
gh api -X PUT repos/matumi-software/koppel/private-vulnerability-reporting
```
