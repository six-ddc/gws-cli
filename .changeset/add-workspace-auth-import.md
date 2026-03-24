---
"@googleworkspace/cli": minor
---

Add `gws auth login-workspace` command for zero-setup authentication via a Cloud Function proxy (no GCP project or client_secret needed). Uses the same OAuth flow as the gemini-cli-extensions/workspace extension — the Cloud Function handles token exchange and refresh server-side.
