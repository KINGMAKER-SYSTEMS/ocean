import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { truncateToWidth, visibleWidth } from "@earendil-works/pi-tui";

const THEME_NAME = "ocean-barnacle";
const STATUS_KEY = "ocean-barnacle";
const WIDGET_KEY = "ocean-barnacle-widget";

const AGENTS: Record<string, { name: string; glyph: string; color: string; lane: string }> = {
  NAV: { name: "Navigator", glyph: "🧭", color: "accent", lane: "orchestration" },
  ANK: { name: "Anchor", glyph: "⚓", color: "warning", lane: "source-of-truth" },
  KEL: { name: "Keel", glyph: "⛵", color: "success", lane: "runtime" },
  SNR: { name: "Sonar", glyph: "◉", color: "mdLink", lane: "search" },
  BCN: { name: "Beacon", glyph: "✦", color: "syntaxNumber", lane: "health" },
};

function cwdThemeDir(cwd: string) {
  return path.join(cwd, ".pi", "themes");
}

function tmuxFormat(format: string): string | undefined {
  if (!process.env.TMUX || !process.env.TMUX_PANE) return undefined;
  try {
    return execFileSync("tmux", ["display-message", "-p", "-t", process.env.TMUX_PANE, format], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch {
    return undefined;
  }
}

function currentWindow(): string | undefined {
  return tmuxFormat("#{window_name}");
}

function inferCallsign(): string {
  const raw = process.env.RT_NAS_AGENT_CALLSIGN || process.env.TIDES_MESH_CALLSIGN || process.env.PI_AGENT_NAME || currentWindow() || "NAV";
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
  if (process.env.OCEAN_BARNACLE_MODE === "0") return false;
  if (process.env.OCEAN_BARNACLE_MODE === "1" || process.env.RT_NAS_AGENT_NAME || process.env.RT_NAS_AGENT_CALLSIGN) return true;
  const win = currentWindow();
  return Boolean(win && ["NAV", "ANK", "KEL", "SNR", "BCN", "README"].some((name) => win === name || win.includes(name)));
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
  const counts: Record<string, number> = { todo: 0, in_progress: 0, blocked: 0, review: 0, done: 0 };
  try {
    const plan = JSON.parse(fs.readFileSync(planFile, "utf8"));
    if (typeof plan.task_count === "number" || typeof plan.completed_count === "number") {
      counts.done = Number(plan.completed_count || 0);
      counts.todo = Math.max(0, Number(plan.task_count || 0) - counts.done);
    }
    const tasks = Array.isArray(plan.tasks) ? plan.tasks : [];
    for (const task of tasks) {
      const status = String(task.status || "todo");
      counts[status] = (counts[status] || 0) + 1;
    }
  } catch {
    // no board yet
  }
  return { agents, counts };
}

function reefLine(width: number, theme: any, callsign: string): string {
  const agent = AGENTS[callsign] || AGENTS.NAV;
  const left = theme.fg(agent.color, `${agent.glyph} ${agent.name}-${callsign}`);
  const middle = theme.fg("muted", " 𓇼 barnacle mode 𓆉 ocean bedrock 𓇼 ");
  const right = theme.fg("dim", agent.lane);
  const pad = " ".repeat(Math.max(1, width - visibleWidth(left) - visibleWidth(middle) - visibleWidth(right)));
  return truncateToWidth(left + middle + pad + right, width);
}

function headerLines(theme: any, callsign: string): string[] {
  const agent = AGENTS[callsign] || AGENTS.NAV;
  const c = (token: string, text: string) => theme.fg(token, text);
  return [
    c("borderMuted", "        .  °  .     🫧        .     °"),
    c("accent", "   ╭────────────────────────────────────────╮"),
    c("accent", "   │") + c(agent.color, ` ${agent.glyph}  OCEAN BEDROCK CREW · ${agent.name}-${callsign}`) + c("accent", " │"),
    c("accent", "   ╰──") + c("warning", "⚓") + c("borderMuted", "──𓇼──𓆉──barnacle-encrusted shell──𓇼──") + c("accent", "╯"),
  ];
}

export default function oceanBarnacleMode(pi: ExtensionAPI) {
  let active = false;

  pi.on("resources_discover", async (event) => {
    const dir = cwdThemeDir(event.cwd);
    return fs.existsSync(dir) ? { themePaths: [dir] } : undefined;
  });

  function install(ctx: any, notify = false) {
    if (!ctx.hasUI || !shouldActivate()) return false;
    active = true;
    const callsign = inferCallsign();
    const agentName = inferAgentName(callsign);
    const agent = AGENTS[callsign] || AGENTS.NAV;
    const result = ctx.ui.setTheme(THEME_NAME);

    ctx.ui.setTitle(`Ocean Bedrock · ${agentName}-${callsign}`);
    ctx.ui.setHeader((_tui: any, theme: any) => ({
      render(width: number): string[] {
        return headerLines(theme, callsign).map((line) => truncateToWidth(line, width));
      },
      invalidate() {},
    }));

    ctx.ui.setWorkingMessage(`${agent.glyph} ${agentName}-${callsign} riding the tide`);
    ctx.ui.setWorkingIndicator({
      frames: [
        ctx.ui.theme.fg("dim", "·"),
        ctx.ui.theme.fg("muted", "≈"),
        ctx.ui.theme.fg("accent", "◌"),
        ctx.ui.theme.fg(agent.color, "●"),
        ctx.ui.theme.fg("accent", "◌"),
        ctx.ui.theme.fg("muted", "≈"),
      ],
      intervalMs: 110,
    });

    ctx.ui.setStatus(STATUS_KEY, ctx.ui.theme.fg(agent.color, `${agent.glyph} ${agentName}-${callsign} · ${agent.lane}`));
    ctx.ui.setWidget(WIDGET_KEY, (_tui: any, theme: any) => ({
      render(width: number): string[] {
        const { agents, counts } = countCrew(ctx.cwd || process.cwd());
        const line1 = reefLine(width, theme, callsign);
        const line2Raw = `${theme.fg("success", `${counts.done || 0} done`)}  ${theme.fg("warning", `${counts.in_progress || 0} active`)}  ${theme.fg("error", `${counts.blocked || 0} blocked`)}  ${theme.fg("muted", `${counts.todo || 0} todo`)}  ${theme.fg("accent", `${agents} crew pings`)}  ${theme.fg("dim", "shells sealed · secrets stay below deck")}`;
        return [line1, truncateToWidth(line2Raw, width)];
      },
      invalidate() {},
    }), { placement: "belowEditor" });

    ctx.ui.setFooter((tui: any, theme: any, footerData: any) => {
      const unsub = footerData.onBranchChange?.(() => tui.requestRender());
      return {
        dispose: unsub,
        invalidate() {},
        render(width: number): string[] {
          const { agents, counts } = countCrew(ctx.cwd || process.cwd());
          const left = theme.fg(agent.color, `${agent.glyph} ${agentName}-${callsign}`);
          const mid = `${theme.fg("success", `${counts.done || 0}✓`)} ${theme.fg("warning", `${counts.in_progress || 0}↯`)} ${theme.fg("error", `${counts.blocked || 0}!`)} ${theme.fg("accent", `${agents} crew`)}`;
          const branch = footerData.getGitBranch?.() ? `  ${footerData.getGitBranch()}` : "";
          const right = theme.fg("muted", `${ctx.model?.id || "model"}${branch}`);
          const pad1 = " ".repeat(Math.max(1, Math.floor((width - visibleWidth(left) - visibleWidth(mid) - visibleWidth(right)) / 2)));
          const pad2 = " ".repeat(Math.max(1, width - visibleWidth(left) - visibleWidth(mid) - visibleWidth(right) - visibleWidth(pad1)));
          return [truncateToWidth(left + pad1 + mid + pad2 + right, width)];
        },
      };
    });

    if (notify) {
      ctx.ui.notify(result?.success === false ? `Ocean Barnacle mode active, theme failed: ${result.error}` : "Ocean Barnacle mode active", result?.success === false ? "warning" : "info");
    }
    return true;
  }

  function uninstall(ctx: any) {
    active = false;
    ctx.ui.setStatus(STATUS_KEY, undefined);
    ctx.ui.setWidget(WIDGET_KEY, undefined);
    ctx.ui.setFooter(undefined);
    ctx.ui.setHeader(undefined);
    ctx.ui.setWorkingMessage();
    ctx.ui.setWorkingIndicator();
    ctx.ui.setTitle("pi");
  }

  pi.on("session_start", async (_event, ctx) => {
    install(ctx, false);
  });

  pi.registerCommand("ocean-barnacle", {
    description: "Toggle Ocean Bedrock's nautical barnacle TUI mode.",
    handler: async (_args, ctx) => {
      if (active) {
        uninstall(ctx);
        ctx.ui.notify("Ocean Barnacle mode disabled", "info");
      } else {
        process.env.OCEAN_BARNACLE_MODE = "1";
        install(ctx, true);
      }
    },
  });
}
