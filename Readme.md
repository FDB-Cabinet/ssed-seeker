Seed Seeker
===========

Seed Seeker is a small CLI tool that runs FoundationDB simulation workloads with different random seeds to find faulty seeds. When a faulty run is detected, Seed Seeker collects logs and, if GitLab credentials are configured, automatically opens a GitLab issue with the relevant artifacts attached.

What it does
- Launches `fdbserver` in simulation mode on a given `.toml`/workload file.
- Iterates across random or user-provided seeds (optionally bounded by a max iteration count).
- Runs multiple seeds in parallel to speed up discovery.
- For non‑successful runs, collects stdout, stderr, and trace logs, filters Rust layer errors, and:
  - If GitLab credentials are provided, files a GitLab issue with attachments.
  - If no GitLab credentials are provided, exits non‑zero to signal a faulty seed.

Prerequisites
- FoundationDB installed locally (or at least the `fdbserver` binary available).
  - Default path used: `/usr/sbin/fdbserver`.
  - If your binary lives elsewhere, provide `--fdbserver-path`.
- Access to a GitLab project where the tool can create issues.
  - A personal access token or project access token with the permission to create issues and upload files.
- libjq (jq C library) installed; required by `jq-rs` to filter JSON logs.
  - Linux:
    - Debian/Ubuntu: `sudo apt-get install libjq1 libjq-dev`
    - Fedora/RHEL: `sudo dnf install jq-libs jq-devel`
    - Arch: `sudo pacman -S jq` (provides libjq)
  - macOS: `brew install jq` (provides libjq)
  - Note: When building from source, ensure the development headers (`libjq-dev`/`jq-devel`) are installed so the crate can link.
  - Oniguruma (libonig) development package may be required because jq links against it for regex support.
    - Debian/Ubuntu: `sudo apt-get install libonig-dev`
    - Fedora/RHEL: `sudo dnf install oniguruma-devel`
    - Arch: `sudo pacman -S oniguruma`
    - macOS (if needed): `brew install oniguruma`
  - IMPORTANT: If the build fails with "Unable to find libjq. Try setting `JQ_LIB_DIR` to specify the location of the lib.", set `JQ_LIB_DIR` to the directory containing your `libjq` shared library.
    - Linux (Ubuntu/Debian typical): `export JQ_LIB_DIR=/usr/lib/x86_64-linux-gnu`
    - macOS (Homebrew on Apple Silicon): `export JQ_LIB_DIR=/opt/homebrew/lib`
    - macOS (Homebrew on Intel): `export JQ_LIB_DIR=/usr/local/lib`
    - You can verify the location by running `ls $(pkg-config --variable=libdir jq)` if `pkg-config` is set up.
- Rust toolchain (if building from source): https://rustup.rs

Environment and configuration
- Environment variables (can be supplied via a `.env` file thanks to dotenv):
  - `GITLAB_TOKEN` (optional): GitLab token with issue creation and upload permissions. Required only if you want Seed Seeker to automatically create GitLab issues and upload artifacts.
  - `GITLAB_URL` (optional): GitLab host, defaults to `gitlab.com`.
  - `GITLAB_PROJECT_ID` (optional): Numeric project ID in GitLab. Required only together with a token to enable automatic issue creation.
- Logging:
  - The CLI uses `tracing_subscriber`. You can control verbosity with `RUST_LOG`, e.g. `RUST_LOG=info` or `RUST_LOG=debug`.

Installation
- From source (in this repository):
  - cargo install --path .
- From the project directory for local development:
  - cargo build --release
  - The resulting binary will be at `target/release/seed-seeker`.

CLI usage
The CLI options are:
- --fdbserver-path <PATH>
  - Path to the `fdbserver` binary.
  - Default: `/usr/sbin/fdbserver`.
- -f, --test-file <FILE>
  - Path to the FoundationDB simulation test/workload file to run.
  - Required.
- --max-iterations <N>
  - Maximum number of iterations/seeds to run. If omitted, runs indefinitely (or until user-provided seeds are exhausted).
- --token <TOKEN>
  - GitLab token to use. If not set, read from `GITLAB_TOKEN`.
  - Optional; required only if you want automatic GitLab issue creation and uploads.
  - Env: `GITLAB_TOKEN`.
- --gitlab-url <HOST>
  - GitLab host (no protocol), e.g. `gitlab.com` or `gitlab.example.com`.
  - Default: `gitlab.com`.
  - Env: `GITLAB_URL`.
- --gitlab-project-id <ID>
  - Numeric GitLab project ID where issues should be created.
  - Optional; required only together with `--token` to enable automatic issue creation.
  - Env: `GITLAB_PROJECT_ID`.
- --commit-id <SHA>
  - Optional commit ID to include in the created issue for context.
- --seed-file <PATH>
  - Path to a file containing seeds, one per line.
- --seeds <SEED[,SEED,...]>
  - Comma‑separated list of seeds to test.
- --chunk-size <N>
  - Number of seeds to run in parallel. Default (if omitted): 10.
- --fail-fast
  - Stop the run after the first faulty seed is found. With GitLab configured, an issue will be created for that seed before exiting; without GitLab, the stdout is printed (if available) and the program exits non‑zero.
- --timeout-secs <SECONDS>
  - Timeout per seed in seconds. The simulation process will be terminated after this period.
  - Default: 120.
  - Env: `TIMEOUT_SECS`.

Notes on seed sources
- You can supply seeds via `--seeds`, `--seed-file`, or let Seed Seeker generate random seeds.
- If both `--seeds` and `--seed-file` are provided, the two sets are merged; both will be executed.
- When providing a file via `--seed-file`, it should contain one unsigned integer per line.

Behavior and outputs
- Successful run (exit code 0): the seed is considered clean; nothing is filed.
- Faulty run (non‑zero exit):
  - Seed Seeker scans collected JSON trace logs and extracts entries with `Layer == "Rust"` and `Severity == "40"` for quick inspection.
  - If GitLab credentials are configured (token + project ID):
    - It uploads three artifacts to GitLab via the project upload API:
      - Full stdout of the simulation.
      - Full stderr of the simulation.
      - A compressed archive of the entire logs directory.
    - An issue titled `Investigate Faulty Seed #<seed>` is created with links to the uploaded artifacts and the filtered log content embedded.
    - If `--fail-fast` is provided, the program exits immediately after creating the issue for the faulty seed.
  - If GitLab credentials are NOT configured:
    - No issue is created and no artifacts are uploaded.
    - The stdout and stderr of the faulty run (if available) are printed, along with the filtered layer errors (Rust, Severity 40) extracted from the JSON logs, before exiting.
    - The program exits with a non‑zero code as soon as a faulty seed is detected.
    - Note: logs are kept in a temporary directory during execution and are cleaned up when the process exits. Configure GitLab to preserve artifacts automatically.
- Per‑seed timeout: each simulation is given up to `--timeout-secs` (default 120s). On timeout the process is terminated, a warning is logged, and the run continues with other seeds (no issue is created for timeouts).

Examples
1) Run random seeds against a workload, limit to 100 iterations, 10 in parallel
   - RUST_LOG=info \
     GITLAB_TOKEN="<your-token>" \
     GITLAB_PROJECT_ID=123456 \
     seed-seeker \
       -f /path/to/workload.toml \
       --max-iterations 100

2) Use a custom fdbserver path and a self‑hosted GitLab
   - RUST_LOG=debug \
     GITLAB_TOKEN="<token>" \
     GITLAB_URL=gitlab.example.com \
     GITLAB_PROJECT_ID=42 \
     seed-seeker \
       --fdbserver-path /opt/foundationdb/bin/fdbserver \
       -f ./workload.toml \
       --max-iterations 50 \
       --chunk-size 8

3) Provide an explicit list of seeds
   - GITLAB_TOKEN="<token>" GITLAB_PROJECT_ID=123 \
     seed-seeker -f ./workload.toml --seeds 101,202,303 --chunk-size 3

4) Load seeds from a file
   - echo -e "11\n22\n33" > seeds.txt
   - GITLAB_TOKEN="<token>" GITLAB_PROJECT_ID=123 \
     seed-seeker -f ./workload.toml --seed-file ./seeds.txt

5) Include a commit id for traceability
   - GITLAB_TOKEN="<token>" GITLAB_PROJECT_ID=123 \
     seed-seeker -f ./workload.toml --commit-id 9b1fc0a --max-iterations 25

FoundationDB requirement
- Seed Seeker runs `fdbserver` in simulation mode; it requires a working FoundationDB installation.
- Ensure the `fdbserver` binary is installed and accessible to the user running the tool.
- Default lookup path is `/usr/sbin/fdbserver`; override with `--fdbserver-path` if needed.

Exit codes
- The CLI exits non‑zero on internal errors (e.g., invalid arguments, I/O errors, GitLab API failures).
- With GitLab configured (token + project ID): faulty simulations cause issue creation; the process continues with other seeds unless `--fail-fast` is specified (in which case the program exits after creating the issue).
- Without GitLab configured: the process exits non‑zero immediately when the first faulty seed is detected (no issue is created); the stdout of the faulty run is printed if available.

Troubleshooting
- Cannot find `fdbserver`:
  - Verify FoundationDB is installed; specify `--fdbserver-path`.
- GitLab 401/403 errors:
  - Check `GITLAB_TOKEN` scope/permissions and project membership.
- No issues created despite failures:
  - Ensure the GitLab project ID is correct and the token has permission to create issues and uploads.
- Too slow:
  - Increase `--chunk-size` to run more seeds concurrently (mind CPU/IO limits).

License
- BSD-3-Clause

Contributing
- Issues and PRs are welcome. Please include reproduction details and logs where relevant.
