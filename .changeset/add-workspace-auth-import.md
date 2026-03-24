---
"@googleworkspace/cli": minor
---

Simplify `gws auth login` to use Cloud Function proxy by default (no GCP project or client_secret needed). Uses the same OAuth flow as the gemini-cli-extensions/workspace extension — the Cloud Function handles token exchange and refresh server-side. Advanced auth options (setup, own-client, custom scopes) are hidden from help but still available.
