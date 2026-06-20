import fs from "node:fs";
import path from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const THEME_NAME = "ocean-bedrock";
const STATUS_KEY = "ocean-bedrock";

const AGENTS: Record<string, { name: string; glyph: string; color: string }> = {
  NAV: { name: "Navigator", glyph: "🧭", color: "accent" },
  ANK: { name: "Anchor", glyph: "⚓", color: "warning" },
  KEL: { name: "Keel", glyph: "⛵", color: "success" },
  SNR: { name: "Sonar", glyph: "◉", color: "mdLink" },
  BCN: { name: "Beacon", glyph: "✦", color: "syntaxNumber" },
};

function inferCallsign(): string {
  const raw = process.env.RT_NAS_AGENT_CALLSIGN || process.env.PI_AGENT_NAME || "NAV";
  const upper = raw.toUpperCase();
  for (const key of Object.keys(AGENTS)) {
    if (upper === key || upper.includes(key) || upper.includes(AGENTS[key]!.name.toUpperCase())) return key;
  }
  return "NAV";
}

function inferAgentName(callsign: string): string {
  return process.env.RT_NAS_AGENT_NAME || AGENTS[callsign]?.name || "Navigator";
}

function shouldActivate(): boolean {
  if (process.env.OCEAN_BEDROCK_MODE === "0") return false;
  return Boolean(process.env.OCEAN_BEDROCK_MODE === "1" || process.env.OCEAN_BARNACLE_MODE === "1" || process.env.RT_NAS_AGENT_NAME || process.env.RT_NAS_AGENT_CALLSIGN);
}

function countCrew(cwd: string) {
  const liveDir = path.join(cwd, ".pi", "messenger", "live", "agents");
  let agents = 0;
  try {
    agents = fs.readdirSync(liveDir).filter((file) => file.endsWith(".json")).length;
  } catch {
    agents = 0;
  }

  const planFile = path.join(cwd, ".pi", "messenger", "crew", "plan.json");
  const counts: Record<string, number> = { todo: 0, in_progress: 0, blocked: 0, done: 0 };
  try {
    const plan = JSON.parse(fs.readFileSync(planFile, "utf8"));
    counts.done = Number(plan.completed_count || 0);
    counts.todo = Math.max(0, Number(plan.task_count || 0) - counts.done);
    for (const task of Array.isArray(plan.tasks) ? plan.tasks : []) {
      const status = String(task.status || "todo");
      counts[status] = (counts[status] || 0) + 1;
    }
  } catch {
    // no board yet
  }
  return { agents, counts };
}

export default function oceanBedrockMode(pi: ExtensionAPI) {
  let active = false;

  function install(ctx: any, notify = false) {
    if (!ctx.hasUI || !shouldActivate()) return false;
    active = true;
    const callsign = inferCallsign();
    const agentName = inferAgentName(callsign);
    const agent = AGENTS[callsign] || AGENTS.NAV;
    const result = ctx.ui.setTheme(THEME_NAME);

    // Clean up the too-loud v1 chrome if hot-reloading over it.
    ctx.ui.setHeader(undefined);
    ctx.ui.setWidget("ocean-bedrock-widget", undefined);
    ctx.ui.setWidget("ocean-barnacle-widget", undefined);

    ctx.ui.setTitle(`Ocean · ${agentName}-${callsign}`);
    ctx.ui.setWorkingMessage(`${agent.glyph} ${agentName}-${callsign} holding station`);
    ctx.ui.setWorkingIndicator({
      frames: [
        ctx.ui.theme.fg("dim", "·"),
        ctx.ui.theme.fg("muted", "≈"),
        ctx.ui.theme.fg(agent.color, "●"),
        ctx.ui.theme.fg("muted", "≈"),
      ],
      intervalMs: 130,
    });

    ctx.ui.setStatus(STATUS_KEY, ctx.ui.theme.fg(agent.color, `${agent.glyph} ${callsign}`));
    ctx.ui.setFooter(undefined);

    if (notify) ctx.ui.notify(result?.success === false ? `Ocean Bedrock theme failed: ${result.error}` : "Ocean Bedrock mode active (clean)", result?.success === false ? "warning" : "info");
    return true;
  }

  function uninstall(ctx: any) {
    active = false;
    ctx.ui.setStatus(STATUS_KEY, undefined);
    ctx.ui.setFooter(undefined);
    ctx.ui.setHeader(undefined);
    ctx.ui.setWidget("ocean-bedrock-widget", undefined);
    ctx.ui.setWidget("ocean-barnacle-widget", undefined);
    ctx.ui.setWorkingMessage();
    ctx.ui.setWorkingIndicator();
    ctx.ui.setTitle("pi");
  }

  pi.on("session_start", async (_event, ctx) => install(ctx, false));

  pi.registerCommand("ocean-bedrock", {
    description: "Toggle Ocean Bedrock's clean nautical TUI mode.",
    handler: async (_args, ctx) => {
      if (active) {
        uninstall(ctx);
        ctx.ui.notify("Ocean Bedrock mode disabled", "info");
      } else {
        process.env.OCEAN_BEDROCK_MODE = "1";
        install(ctx, true);
      }
    },
  });
}
