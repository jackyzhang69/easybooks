# EasyBooks plugin — agent notes

## Official plugin release

Tag `plugin-v*` only after `python3 scripts/official-plugin/preflight-official-plugin-release.py --plugin-id easybooks` passes. CI `.github/workflows/publish.yml` signs Mac binaries and writes `jackyzhang69/plugins` through `publish-official-plugin.py`. There is no local marketplace publisher. Mac CI/publish jobs use capability labels `[self-hosted, macOS, ARM64]` only — do not pin `easybooks-plugin-signing`. Keep the publish deploy concurrency group.

Runtime config is `~/.jackyzhang.app/easybooks/config.json`. Portal token stays in the shared user slot.
