# file-courier — agent instructions

Provider-agnostic Ocean courier on the **Slack route**. The shared transport
(`../transport/slack.py`) is the only thing that talks to Slack; the agent loop
only orchestrates and confirms.

See [`CLAUDE.md`](CLAUDE.md) for the full operating protocol — it is provider-
neutral and applies regardless of which model (DeepSeek/Anthropic/OpenAI/Google)
drives this agent under the Ocean runtime.

Principles:
- The route owns transport: auth, the 3-step file upload, rate limits, retries,
  threading. Never hand-roll Slack API calls in the agent loop.
- Destination is supplied at run time as a Slack channel link. Nothing hardcoded.
- The bot must be a member of the target channel (`not_in_channel` → invite it).
- Always resolve + confirm the destination (bot identity / channel name) with the
  operator before sending into a shared channel.
- Secrets come from env / `courier.env`, never from code or commits.

Self-test (offline link parsing, no token needed):
```bash
python3 -c "import sys;sys.path.insert(0,'../transport');import slack;\
print(slack.parse_link('https://x.slack.com/archives/C0123ABCD/p1700000000000100'))"
```
