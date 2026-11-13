time: [17:29] [11-13-26] [America/New_York]
agent: [claude] [opus 5.5]
worktree: [claude/model-registry-20261113]
type: [feature-request]
area: [backend] [infra]

The owner asked for every provider's new models in the Ocean registry and for
the daemon to actually work on smaths-mini. The daemon was the first problem:
it accepted connections on 127.0.0.1:4780 and answered none. The machine was
at a sustained load average near 300, and the LaunchAgent ran as
`ProcessType=Background`, the launchd class macOS throttles, so the daemon
(and even its bash launcher) got almost no CPU. Switching the installed plist
to `Interactive` brought `/v1/models` back in about 3 seconds; the repo plist
now ships `Interactive` so the installer does not revert it.

The catalog was refreshed against each provider's live `/models` list and a
real request per new model. Live-verified: Claude Fable 5.2 (adaptive only)
and Haiku 5.5 (adaptive, can disable), Codex GPT-6.1 Terra plus the 500K Codex
window for the 6.1 generation, GLM 5.1 / 5 / 5 Turbo / 4.5 Air. GPT-5.5
(Codex 400s it), Haiku 4.5 and Gemini 2.0 Flash left the menu and stay
routable; Fable, Haiku and Gemini aliases move to the newest ids. The
DeepSeek, MiniMax, Moonshot and Kimi-coding keys in the local auth file all
return 401 and there is no Gemini key, so `deepseek-flash`, MiniMax M3 / M3.1
Flash, Kimi K3.1 Pro (+ highspeed) and Gemini 4 Argon / Carbon / Hydrogen /
3.8 Flash were added from provider docs only. GLM 5.3 FlashX is listed but
not included in the owner's plan, so it stays off the menu.

Validation: `cargo test -p ocean-providers -p ocean-protocol -p ocean-agent
-p ocean-daemon -p ocean-acp` all green (new refresh test included). An
independent reviewer subagent found one defect (provider-pinned MiniMax M3
got a 200K window), fixed with a test.
