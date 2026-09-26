## Summary

<!-- What changed and why. Keep scope tight. -->

## CI runners

Mac jobs use capability labels only:

`runs-on: [self-hosted, macOS, ARM64]`

Do not pin `easybooks-plugin-signing` or any other named runner. Leave deploy concurrency groups in `publish.yml` unchanged.
