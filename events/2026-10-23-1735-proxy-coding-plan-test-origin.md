time: [17:35] [10-23-26]
agent: [claude code] [claude-fable-5-1]
worktree: [fix/proxy-coding-plan-test-origin] [~/dev/oceanwork/wt-proxyfix]
type: [bug-report]
area: [testing]

Repaired the proxy test suite on main, which went red when two same-day ports met: ocean#157's auth-off cross-site gate correctly refuses a mutation that names a foreign Origin, and ocean#159's `coding_plan_routes_forward_only_the_server_side_key` sent exactly that shape in its no-login branch to prove the header is stripped. The test now asserts the gate's 403 `cross_site_mutation_refused` for the auth-off mutations with a foreign source, then re-sends them headerless (admitted as "no browser source") to keep proving the lane attaches no key; the login branch and the reads keep the hostile headers and the strip assertions. Also made ocean#171's constant upload-ceiling comparison a `const _: () = assert!` so clippy `-D warnings` passes on the proxy test target. Hosted CI only compiles, so the proxy suite must be run locally after any proxy change; 72/72 now, fmt and clippy clean, `cargo build --locked` clean.
